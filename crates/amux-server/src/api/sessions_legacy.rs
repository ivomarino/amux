//! GET /api/sessions — the PYTHON-SHAPED session list (RR-0075 enabler).
//!
//! The alias layer rewrites legacy PATHS, but the SPA also expects the
//! Python RESPONSE SHAPE: a bare array of `{name, status, preview, ...}`.
//! The modern /api/workers envelope (items/total, display_name, typed
//! state) is right for new clients; this projection is what lets the
//! 44k-line dashboard render workers today, unchanged. It is registered
//! BEFORE the rewrite middleware so it wins over the path alias.

use super::AppState;
use crate::backend::tmux::pane_target;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// WorkerState -> the Python status vocabulary the SPA's badges render.
fn python_status(state_json: &str) -> &'static str {
    // state_json is the row's JSON WorkerState; match on the tag cheaply.
    if state_json.contains("\"active\"") {
        "active"
    } else if state_json.contains("\"idle\"") {
        "idle"
    } else if state_json.contains("\"waiting\"") {
        "waiting"
    } else if state_json.contains("\"rate_limited\"") {
        "rate-limited"
    } else if state_json.contains("\"error\"") {
        "error"
    } else if state_json.contains("\"starting\"") {
        "starting"
    } else {
        "" // stopped renders as blank in the Python list
    }
}

// ---- status derivation (AMUX-2589) ---------------------------------------
//
// Python's `status` is its scanner's judgment (pane regex) overridden by a
// fresh self-report (amux-server.py:20201-20263). The Rust server runs no
// scanner (D1: scrapers are the deviation, not the goal), so the honest
// equivalents are, in Python-precedence order:
//   base:  the Python scanner's own LAST PERSISTED judgment — the
//          session.working/idle/waiting transition it writes to
//          `session_events` (py:20268-20270, the D1 report-endpoint shape:
//          a durable store the producer already writes) — guarded against
//          staleness (pre-restart events discarded; an `active` with no
//          pane output for AMUX_ACTIVE_HEARTBEAT_S is not active);
//          falling back to tmux activity (<60s = active, else idle).
//   over:  self_report when fresh, with Python's ASYMMETRIC freshness
//          (py:20233-20263): `idle` does not decay (the only exit is a
//          prompt, which fires UserPromptSubmit -> a new report; window
//          AMUX_HOOKS_LIVE_IDLE_S=86400), `active`/`waiting` do
//          (AMUX_HOOKS_LIVE_S=1800), and a stale `active` report (older
//          than the heartbeat, AMUX_ACTIVE_HEARTBEAT_S=120) never
//          overrides — a long turn is byte-identical to a wedged one.
//   last:  CONTRADICTION — physical evidence overrides a stale `idle`
//          (AMUX-2646, below).
//   "" :   not running.
//
// AMUX-2646 — "it is running but says idle". The asymmetric window above
// says an `idle` report never decays, on the reasoning that "the only exit
// from idle is a prompt, and every prompt fires UserPromptSubmit". That
// premise is false in at least four reachable ways, and each of them leaves
// a lane permanently mislabelled because nothing else in the derivation
// could ever disagree:
//
//   1. The UserPromptSubmit POST is best-effort (`curl -m 2`, no retry). The
//      server re-execs on every save of its own source on a shared checkout,
//      so a dropped report is routine, not exotic.
//   2. `report_post` then REFUSES every `tool-hook` heartbeat for the rest of
//      the turn ("a heartbeat must not resurrect a finished turn",
//      AMUX-2538) — the one signal that could self-heal is suppressed by
//      design, correctly, for a different reason.
//   3. Anything can write any state for any session over `/report`; a hand
//      -run hook test wrote `{"state":"idle","source":"stop-hook-test"}` onto
//      a LIVE working lane and it stuck for 1076s until a human noticed.
//   4. Work resumes without a prompt: a backgrounded command re-invoking the
//      agent, a resumed session, a hookless provider (gemini/codex) whose
//      stale claude-era report outlives its hooks.
//
// A claim that no evidence can contradict is not a status, it is an axiom.
// So `idle` still survives SILENCE for the full 24h — a parked lane must not
// be re-scraped forever, which is the asymmetry's real purpose — but it does
// not survive CONTRADICTION: a pane that is unambiguously mid-turn AND has
// painted within AMUX_IDLE_CONTRADICTION_S overrides an idle report older
// than that same window. Both halves are required, and the "has painted"
// half is what keeps this from re-reading a parked lane's scrollback: a lane
// that quotes "esc to interrupt" in a transcript it wrote hours ago (a real
// self-block here, AMUX-2642) emits no output, so it is never probed.
//
// KNOWN residual, measured 2026-08-09 against the live fleet (114/116
// exact): Python emits "" for a RUNNING session whose pane shows no
// recognizable agent UI (claude exited to a shell). That cell exists only
// in the pane regex; this derivation reads idle for it. Re-implementing
// the regex would deepen D1, so the residual is documented, not coded away.

fn env_secs(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The whole-fleet pane snapshot, shared by every reader inside the TTL.
///
/// A process global rather than a field on `AppState`: `FleetSignals::load` is
/// a free function called from three places that do not share a handle, and
/// threading one through would be a wider change to files other lanes are
/// editing. The value is a pure cache — dropping it costs one re-capture and
/// changes no verdict.
#[allow(clippy::type_complexity)]
fn pane_cache() -> &'static std::sync::Mutex<(f64, BTreeMap<String, String>)> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<(f64, BTreeMap<String, String>)>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new((0.0, BTreeMap::new())))
}

/// One `tmux list-sessions` line -> (name, last-painted, created).
///
/// Pulled out of `load` so the ACTIVITY RULE is testable without a tmux
/// server: it is the rule that was silently wrong for the whole fleet, and a
/// test that re-types the parse inline would have agreed with whatever it was
/// re-typing. Returns `None` for a line that does not carry at least a name.
fn parse_list_sessions_line(l: &str) -> Option<(&str, Option<i64>, Option<i64>)> {
    let mut it = l.split(':');
    let name = it.next()?;
    if name.is_empty() {
        return None;
    }
    let (a, c, w) = (it.next(), it.next(), it.next());
    // max(session_activity, window_activity) — see `FleetSignals::activity`.
    let last_paint: Option<i64> = a
        .and_then(|x| x.parse().ok())
        .into_iter()
        .chain(w.and_then(|x| x.parse::<i64>().ok()))
        .max();
    Some((name, last_paint, c.and_then(|x| x.parse().ok())))
}

/// Signals the derivation reads, loaded once per request and shared with the
/// board's `stale` computation (`active_python_sessions`) so the two can
/// never disagree about who is working.
pub struct FleetSignals {
    /// tmux session name (`amux-<n>`) -> when its pane last PAINTED, i.e.
    /// `max(#{session_activity}, #{window_activity})`.
    ///
    /// It has to be the max, and that is not a belt-and-braces choice.
    /// `#{session_activity}` does not track pane output for a DETACHED
    /// session, and every amux lane is detached: measured on tmux 3.6a,
    /// 2026-08-09, 60 of 63 live sessions had a `session_activity` more than
    /// 60s older than their `window_activity`, and `amux-rust` — mid-turn,
    /// spinner repainting ~6/s — reported a `session_activity` that had not
    /// moved in 34.5 HOURS (it was still equal to `session_created`).
    ///
    /// Everything downstream read that as silence, so the two places this
    /// derivation consults physical liveness were both dead: the
    /// `now - act < 60` fallback could never say `active`, and the guard that
    /// demotes a stale `active` transition fired for EVERY session on every
    /// request. The fleet's status was therefore whatever the self-reports
    /// said and nothing else — which is precisely why one wrong report could
    /// not be contradicted by anything.
    pub activity: BTreeMap<String, i64>,
    /// tmux session name -> `#{session_created}`.
    pub created: BTreeMap<String, i64>,
    /// Live tmux session names.
    pub running: BTreeSet<String>,
    /// tmux session names whose pane is sitting in a bare SHELL — the tmux
    /// session exists but the agent inside it is gone. `stop` deliberately
    /// leaves the tmux session alive (Python parity), so tmux-existence alone
    /// says "there is a window", not "there is a worker": a stopped lane read
    /// as running=true forever on the card while `/api/sessions/<n>/info`
    /// (which checks the pane) said false. Two answers to one question, and
    /// the card is the one the user is looking at — clicking Stop appeared to
    /// do nothing. Measured 2026-08-09.
    pub shell_only: BTreeSet<String>,
    /// The persisted self-report store (prefs `session_reports`,
    /// amux-server.py:3943) — the same bytes Python hydrates at boot.
    pub reports: serde_json::Value,
    /// session -> (status, ts) of its latest working/idle/waiting transition.
    pub transitions: BTreeMap<String, (String, f64)>,
    /// session -> ts of its latest `session.started` event.
    pub started: BTreeMap<String, f64>,
    /// session name -> raw pane capture, for lanes that PAINTED recently.
    ///
    /// The only physical evidence in this struct: everything else is a claim
    /// somebody wrote down. Populated by [`FleetSignals::capture_panes`] and
    /// read only through [`FleetSignals::pane_of`], which re-applies the same
    /// candidacy predicate the capture used — so a caller that captures more
    /// (the session list, which already has every running pane in hand for
    /// previews) and one that captures less (the board) still derive the same
    /// status for the same lane. A view that disagrees with the mechanism it
    /// describes is worse than no view.
    pub panes: BTreeMap<String, String>,
    pub now: f64,
}

impl FleetSignals {
    pub fn load(conn: &rusqlite::Connection) -> Self {
        let mut activity = BTreeMap::new();
        let mut created = BTreeMap::new();
        let mut running = BTreeSet::new();
        // The Ok() only means the SPAWN worked — tmux exiting non-zero (no
        // server, wrong socket) still lands here with empty stdout, and an
        // empty fleet is indistinguishable from a dead probe (ethos rule 4;
        // live incident 2026-08-09: launchd build served running=0 for 116
        // cards while 62 tmux sessions ran, with nothing in the log).
        //
        // Separator is ':' NOT '\t': under launchd there is no LANG, and in
        // the POSIX locale tmux sanitizes non-printable output chars to '_',
        // so a tab-separated format came back as `name_123_456` and every
        // parse silently missed (the same 2026-08-09 incident — /api/debug/tmux
        // is what caught it). ':' is safe because tmux forbids it in session
        // names (target syntax), and printable chars are never sanitized.
        //
        // `#{window_activity}` is the 4th field because `#{session_activity}`
        // is not a liveness signal for a detached session — see the `activity`
        // field's doc for the measurement. It resolves to the session's
        // CURRENT window; amux creates one window per session, and the agent
        // runs in it.
        let tmux_out = std::process::Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}:#{session_activity}:#{session_created}:#{window_activity}",
            ])
            .output();
        match &tmux_out {
            Ok(o) if !o.status.success() => tracing::warn!(
                status = %o.status,
                stderr = %String::from_utf8_lossy(&o.stderr).trim(),
                "tmux list-sessions failed — fleet will read as not-running"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                "tmux spawn failed — fleet will read as not-running"
            ),
            _ => {}
        }
        if let Ok(o) = tmux_out {
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let Some((n, a, c)) = parse_list_sessions_line(l) else {
                    continue;
                };
                running.insert(n.to_string());
                if let Some(ts) = a {
                    activity.insert(n.to_string(), ts);
                }
                if let Some(ts) = c {
                    created.insert(n.to_string(), ts);
                }
            }
        }
        // ONE extra batched tmux call for the whole fleet (not per session):
        // which panes are a bare shell. `#{pane_current_command}` is the
        // foreground command, so an agent shows as `claude`/`node`/`codex`
        // and a stopped lane shows as `bash`. A session with several panes
        // counts as shell-only only if EVERY pane is a shell.
        let mut shell_only = BTreeSet::new();
        if let Ok(o) = std::process::Command::new("tmux")
            .args(["list-panes", "-a", "-F", "#{session_name}:#{pane_current_command}"])
            .output()
        {
            const SHELLS: [&str; 8] = ["bash", "zsh", "sh", "fish", "dash", "ksh", "tcsh", "csh"];
            let mut any_live: BTreeSet<String> = BTreeSet::new();
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let Some((sess, cmd)) = l.rsplit_once(':') else { continue };
                seen.insert(sess.to_string());
                let cmd = cmd.trim().trim_start_matches('-');
                if !SHELLS.contains(&cmd) {
                    any_live.insert(sess.to_string());
                }
            }
            for s in seen {
                if !any_live.contains(&s) {
                    shell_only.insert(s);
                }
            }
        }
        let reports = conn
            .query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null);
        // Both event queries tolerate the table being absent (a fresh Rust-only
        // AMUX_HOME): no events simply means the activity fallback decides.
        let mut transitions = BTreeMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT session, type, MAX(ts) FROM session_events \
             WHERE type IN ('session.working','session.idle','session.waiting') \
             GROUP BY session",
        ) {
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    let st = match row.1.as_str() {
                        "session.working" => "active",
                        "session.waiting" => "waiting",
                        _ => "idle",
                    };
                    transitions.insert(row.0, (st.to_string(), row.2));
                }
            }
        }
        let mut started = BTreeMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT session, MAX(ts) FROM session_events \
             WHERE type='session.started' GROUP BY session",
        ) {
            if let Ok(rows) =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
            {
                for (s, ts) in rows.flatten() {
                    started.insert(s, ts);
                }
            }
        }
        FleetSignals {
            activity,
            created,
            running,
            shell_only,
            reports,
            transitions,
            started,
            panes: BTreeMap::new(),
            now: chrono::Utc::now().timestamp() as f64,
        }
    }

    /// Is there a WORKER in this tmux session, not merely a tmux session?
    /// This is the question the card is asking, and the one
    /// `session_verbs::is_running` answers for `/info`, restart and delete.
    /// Call this instead of touching `running` directly, or the two answers
    /// drift again.
    pub fn agent_running(&self, tmux_name: &str) -> bool {
        self.running.contains(tmux_name) && !self.shell_only.contains(tmux_name)
    }

    /// How recent must physical evidence be to falsify a reported `idle`, and
    /// how old must that report be before evidence is allowed to falsify it?
    ///
    /// One number for both halves because it is one question: how long after a
    /// lane last spoke do we keep taking its word for it. Inside the window the
    /// report wins (it is the D1 exit — the harness reporting its own state
    /// beats any scrape of it, and this is where the report/repaint race
    /// lives); outside it, a pane that is demonstrably mid-turn wins.
    fn contradiction_window(&self) -> f64 {
        env_secs("AMUX_IDLE_CONTRADICTION_S", 60.0)
    }

    /// Is this lane's pane worth reading, and worth believing?
    ///
    /// ONE predicate, two callers — [`Self::capture_panes`] decides what to
    /// capture and [`Self::pane_of`] decides what to believe. If they ever
    /// drift, the derivation reads a pane the capture never took (or refuses
    /// one it did) and two readers of the same struct disagree about the same
    /// lane. Keeping it here also means a caller CANNOT make a parked lane's
    /// scrollback count as evidence by stuffing the map.
    pub fn pane_probe_candidate(&self, name: &str) -> bool {
        let act = self.activity.get(&format!("amux-{name}")).copied().unwrap_or(0) as f64;
        self.now - act < self.contradiction_window()
    }

    /// Raw pane for a lane whose evidence is admissible: recently painted and
    /// non-empty.
    ///
    /// An EMPTY capture is `None`, never "no markers, therefore idle". A herdr
    /// lane refuses a history read while it is working, so mid-turn its
    /// capture is empty BY DESIGN — reading that as idle would label a working
    /// lane idle, which is this whole bug in a different costume.
    fn pane_of(&self, name: &str) -> Option<&str> {
        if !self.pane_probe_candidate(name) {
            return None;
        }
        let raw = self.panes.get(name)?;
        (!raw.trim().is_empty()).then_some(raw.as_str())
    }

    /// Does the pane show UNAMBIGUOUS work — the evidence that may contradict
    /// a claim of idle?
    ///
    /// Composed from the two detectors that already exist rather than a third
    /// one: `pane_bar_says_generating` (the status bar's `esc to interrupt`,
    /// scoped to the bottom 3 lines by AMUX-2642 so a lane quoting the phrase
    /// cannot self-block) and `detect_claude_status` (the live spinner). Both
    /// answer "is the MAIN turn generating", which is the same question the
    /// steering gate asks — so a lane this reports as working is exactly a
    /// lane that would refuse a mid-turn delivery. A second spelling of the
    /// detector would be a second thing to keep in step with Claude Code's UI.
    fn pane_says_working(&self, name: &str) -> bool {
        let Some(raw) = self.pane_of(name) else {
            return false;
        };
        crate::api::session_verbs::pane_bar_says_generating(raw)
            || crate::api::session_verbs::detect_claude_status(raw) == "active"
    }

    /// Capture the panes that could contradict a report.
    ///
    /// Only lanes that painted inside the contradiction window — typically a
    /// handful of a 60-lane fleet (measured: 4 of 63 on 2026-08-09). A lane
    /// that has not painted cannot be mid-turn: Claude Code repaints its
    /// spinner roughly six times a second.
    ///
    /// Behind a 2s cache, because the typical case is not the one that hurts.
    /// Measured on this box: 4 painting lanes cost 44ms, but a fleet-wide
    /// broadcast puts all 63 lanes in the candidate set and costs 473ms — and
    /// this runs on the board's `stale` computation, which the dashboard polls.
    /// A TTL two orders of magnitude below the contradiction window cannot
    /// change a verdict, and it makes the board and the session list read the
    /// SAME frame rather than two captures 20ms apart.
    pub fn capture_panes(&mut self) {
        let ttl = env_secs("AMUX_PANE_CACHE_TTL_S", 2.0);
        let cache = pane_cache();
        if let Ok(c) = cache.lock() {
            if self.now - c.0 < ttl {
                self.panes = c.1.clone();
                return;
            }
        }
        let names: Vec<String> = self
            .running
            .iter()
            .filter_map(|t| t.strip_prefix("amux-"))
            .filter(|n| self.pane_probe_candidate(n))
            .map(String::from)
            .collect();
        for chunk in names.chunks(12) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|name| {
                    let n = name.clone();
                    std::thread::spawn(move || {
                        let pt = pane_target(&format!("amux-{n}"));
                        let out = std::process::Command::new("tmux")
                            .args(["capture-pane", "-t", &pt, "-p", "-e", "-S", "-30"])
                            .output()
                            .ok()?;
                        Some((n, String::from_utf8_lossy(&out.stdout).trim().to_string()))
                    })
                })
                .collect();
            for h in handles {
                if let Ok(Some((n, raw))) = h.join() {
                    self.panes.insert(n, raw);
                }
            }
        }
        // Store even an EMPTY result: "nothing was painting" is an answer, and
        // a cache that only remembers hits re-probes hardest exactly when the
        // fleet is quiet and there is nothing to find.
        if let Ok(mut c) = pane_cache().lock() {
            *c = (self.now, self.panes.clone());
        }
    }

    /// Lanes whose pane was actually read, with the evidence verdict for each.
    ///
    /// The consistency check (`invariants::checks::status_agrees_with_pane`)
    /// reads its two sides from here and from `derive_status` — one struct,
    /// one capture, one pair of detectors. A check that re-derives either side
    /// its own way is a second implementation that can drift from the thing it
    /// audits, and then its verdict is about itself.
    ///
    /// Excludes shell-only lanes: those have a tmux window and no worker, so
    /// "the card disagrees with the pane" is not a meaningful question for
    /// them.
    pub fn probed_lanes(&self) -> Vec<(String, bool)> {
        self.panes
            .keys()
            .filter(|n| self.agent_running(&format!("amux-{n}")))
            .filter(|n| self.pane_of(n).is_some())
            .map(|n| (n.clone(), self.pane_says_working(n)))
            .collect()
    }

    /// Python's status value for one session (see the derivation note above).
    pub fn derive_status(&self, name: &str, running: bool) -> String {
        if !running {
            return String::new();
        }
        let heartbeat = env_secs("AMUX_ACTIVE_HEARTBEAT_S", 120.0);
        let act = self
            .activity
            .get(&format!("amux-{name}"))
            .copied()
            .unwrap_or(0) as f64;
        let mut status: Option<String> = None;
        if let Some((st, ts)) = self.transitions.get(name) {
            // A transition from before the session's last (re)start describes
            // a previous life — Python never emits a transition out of the ""
            // state, so a restart leaves the old row behind (verified: the
            // guard flipped 1 live mismatch on 2026-08-09).
            if self.started.get(name).copied().unwrap_or(0.0) <= *ts {
                if st == "active" && self.now - act > heartbeat {
                    // An active session paints its pane continuously; silence
                    // past the heartbeat means the transition went stale.
                    status = Some("idle".into());
                } else {
                    status = Some(st.clone());
                }
            }
        }
        // No transition: prefer the PANE over the activity timestamp when the
        // pane is admissible. A timestamp says something painted; the pane
        // says what. `detect_claude_status` returning "" is the documented
        // Python residual (an agentless shell) and reads idle here, as it did
        // before — the fallback below stays for a lane with no readable pane,
        // which after `capture_panes` means a silent one (idle) or a herdr
        // lane mid-turn (empty capture, and `act` is fresh, so: active).
        let mut status = status.unwrap_or_else(|| {
            match self.pane_of(name).map(crate::api::session_verbs::detect_claude_status) {
                Some(v) if v == "active" || v == "waiting" => v,
                Some(_) => "idle".into(),
                None if self.now - act < 60.0 => "active".into(),
                None => "idle".into(),
            }
        });
        // self_report override — Python's exact gate (py:20248-20263).
        let mut idle_report_age: Option<f64> = None;
        if let Some(rep) = self.reports.get(name) {
            let st = rep["state"].as_str().unwrap_or("");
            // ts is time.time() — a FLOAT. as_i64() on it is None, which
            // silently read every report as epoch-0 (the age_s bug).
            let age = self.now - rep["ts"].as_f64().unwrap_or(0.0);
            let stale_active = st == "active" && age > heartbeat;
            let live = age
                < if st == "idle" {
                    env_secs("AMUX_HOOKS_LIVE_IDLE_S", 86400.0)
                } else {
                    env_secs("AMUX_HOOKS_LIVE_S", 1800.0)
                };
            if !stale_active && live && matches!(st, "active" | "idle" | "waiting") {
                status = st.to_string();
                if st == "idle" {
                    idle_report_age = Some(age);
                }
            }
        }
        // CONTRADICTION (AMUX-2646). `idle` survives silence, never
        // contradiction. Fires only when BOTH halves hold: the claim is older
        // than the window (a fresh report is still the authority — D1), and
        // the pane both painted inside the window and shows the main turn
        // generating. It can only ever flip idle -> active, so a missed frame
        // costs a late correction, never a false "busy".
        if status == "idle"
            && idle_report_age.map(|a| a > self.contradiction_window()).unwrap_or(true)
            && self.pane_says_working(name)
        {
            status = "active".into();
        }
        status
    }
}

/// The set of sessions currently `active` — the board's `stale` flag reads
/// this (Python: `_session_prev_status[sess] == "active"`, py:15671-15697).
/// Shares `FleetSignals` with the session list: one derivation, two readers.
pub fn active_python_sessions(conn: &rusqlite::Connection) -> BTreeSet<String> {
    let mut signals = FleetSignals::load(conn);
    // The board must see the same evidence the session list sees, or a lane
    // reads active on one screen and idle on the other. Bounded: only lanes
    // that painted inside the contradiction window are probed.
    signals.capture_panes();
    let mut out = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(amux_home().join("sessions")) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("env") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let running = signals.agent_running(&format!("amux-{name}"));
        if signals.derive_status(name, running) == "active" {
            out.insert(name.to_string());
        }
    }
    out
}

// ---- preview (AMUX-2588) -------------------------------------------------

/// Python's strip_ansi (amux-server.py:20225) — ported verbatim, OSC
/// hyperlink forms included: Claude panes emit `\x1b]8;` constantly, and a
/// simpler regex leaves fragments the intelligibility filter then rejects.
pub(crate) fn strip_ansi(s: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            "\\x1b\\[[0-9;?]*[a-zA-Z]|\\x1b\\]8;[^\\x1b]*\\x1b\\\\|\\x1b\\][^\\x07]*\\x07|\\x1b\\][^\\x1b]*\\x1b\\\\|\\x1b[()][A-Z0-9]|\\x1b[\\x20-\\x2f]*[\\x40-\\x7e]",
        )
        .expect("strip_ansi regex")
    });
    re.replace_all(s, "").into_owned()
}

fn chars_truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python's preview pair (amux-server.py:20224-20316): the scalar is the
/// last non-blank RAW line, sliced to 120 chars THEN stripped (that order is
/// Python's); `preview_lines` is an ARRAY of up to 5 intelligible lines —
/// the SPA calls `.map()` on it (app.js:2602), so the previous line COUNT
/// failed its `&& s.preview_lines.length` check and previews silently never
/// rendered on the Rust side (AMUX-2588).
fn preview_of(raw: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
    let preview = lines
        .iter()
        .rev()
        .map(|l| strip_ansi(&chars_truncate(l, 120)))
        .find(|cl| {
            let lower = cl.to_lowercase();
            let n = cl.chars().count();
            if n <= 2 { return false; }
            if cl.contains("\u{23f5}\u{23f5}")
                || lower.contains("bypass permissions")
                || lower.contains("plan mode")
                || cl.starts_with('\u{276f}')
            {
                return false;
            }
            let alnum = cl.chars().filter(|c| c.is_alphanumeric() || *c == ' ').count();
            n <= 3 || (alnum as f64) / (n as f64) >= 0.3
        })
        .unwrap_or_default();
    let mut intelligible: Vec<String> = Vec::new();
    for l in &lines {
        let cl = strip_ansi(l).trim().to_string();
        if cl.is_empty() {
            continue;
        }
        let lower = cl.to_lowercase();
        if cl.contains("⏵⏵") || lower.contains("bypass permissions") || lower.contains("plan mode")
        {
            continue;
        }
        let n_chars = cl.chars().count();
        let alnum = cl.chars().filter(|c| c.is_alphanumeric() || *c == ' ').count();
        if n_chars > 3 && (alnum as f64) / (n_chars as f64) < 0.3 {
            continue;
        }
        if n_chars <= 2 {
            continue;
        }
        let distinct: BTreeSet<char> = cl.chars().filter(|c| *c != ' ').collect();
        if distinct.len() <= 2 {
            continue;
        }
        intelligible.push(chars_truncate(&cl, 200));
    }
    let preview_lines: Vec<String> = if intelligible.is_empty() {
        // Fallback: last few non-empty stripped lines (spinner/tool output).
        let start = lines.len().saturating_sub(8);
        let cleaned: Vec<String> = lines[start..]
            .iter()
            .map(|l| chars_truncate(strip_ansi(l).trim(), 200))
            .filter(|l| !l.is_empty())
            .collect();
        let s = cleaned.len().saturating_sub(5);
        cleaned[s..].to_vec()
    } else {
        let s = intelligible.len().saturating_sub(5);
        intelligible[s..].to_vec()
    };
    (preview, preview_lines)
}

/// Saved-log tail for a STOPPED session (py:20218-20223): last 16KB of
/// ~/.amux/logs/<name>.log, last 30 lines.
fn stopped_session_raw(name: &str) -> String {
    let p = amux_home().join("logs").join(format!("{name}.log"));
    let Ok(mut f) = std::fs::File::open(&p) else {
        return String::new();
    };
    use std::io::{Read, Seek, SeekFrom};
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if size > 16_384 {
        let _ = f.seek(SeekFrom::Start(size - 16_384));
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(30);
    lines[start..].join("\n")
}

// ---- misc shared helpers -------------------------------------------------

fn amux_home() -> std::path::PathBuf {
    std::env::var("AMUX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
        })
}

/// ~/.amux/sessions/<name>.meta.json (py:_load_meta) — last_send,
/// last_started, task_summary live here.
fn load_meta(name: &str) -> serde_json::Value {
    let p = amux_home().join("sessions").join(format!("{name}.meta.json"));
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// The legacy array as a JSON string, shared by the GET handler and the
/// SSE `sessions` pushes (one serializer, two transports).
pub fn legacy_sessions_array(store: &crate::db::SharedStore) -> anyhow::Result<String> {
    let conn = store.read()?;
    let arr = build_array(&conn)?;
    Ok(serde_json::to_string(&arr)?)
}

pub async fn list_sessions_legacy(State(state): State<AppState>) -> Response {
    let conn = match state.store.read() {
        Ok(c) => c,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    };
    match build_array(&conn) {
        Ok(arr) => Json(serde_json::Value::Array(arr)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ---- POST /api/sessions — CREATE a fleet worker --------------------------
//
// The cutover carried GET across and left POST behind, so the dashboard's
// "New worker" dialog has been 405ing: the toast said "Create failed: error
// 405" and the dialog stayed open, which reads as the Create button doing
// nothing. `POST /api/workers` is NOT the same thing — it inserts a row in
// the `workers` table, a different substrate from the ~/.amux/sessions/*.env
// registry this list (and tmux, and every session verb) reads, so a worker
// created there is invisible to the fleet.
//
// A fleet worker IS its env file. This writes exactly the file the Python
// server wrote (`# updated:` header, K="V", 0600 atomic) and nothing else —
// `/start` does the rest, as it already does for a duplicated session.

/// Python's sanitizer, same as `duplicate`'s: anything outside
/// `[A-Za-z0-9_-]` becomes `-`, so a name can never escape the sessions dir.
fn sanitize_session_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect()
}

/// `# updated:` header + K="V" lines, 0600, atomic rename — byte-compatible
/// with `EnvFile::write` in session_verbs (which is private to that module;
/// duplicating ~15 lines here is cheaper than widening a file another lane is
/// actively editing).
fn write_env_file(path: &std::path::Path, pairs: &[(&str, String)]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut out = format!(
        "# updated: {}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f")
    );
    for (k, v) in pairs {
        out.push_str(&format!("{k}=\"{v}\"\n"));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("env"),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub async fn create_session_legacy(
    State(_state): State<AppState>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    let s = |k: &str| {
        body.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let raw_name = s("name");
    if raw_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "name is required"})),
        )
            .into_response();
    }
    let name = sanitize_session_name(&raw_name);
    if name.is_empty() || name.starts_with('.') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("'{raw_name}' is not a usable worker name")})),
        )
            .into_response();
    }
    // A worktree create needs `git worktree add` + branch bookkeeping that
    // does not exist here yet. REFUSE loudly rather than create a plain
    // worker and let the user believe they got an isolated checkout — a
    // silently-ignored option is the failure mode this whole sweep is about.
    if body.get("worktree").and_then(serde_json::Value::as_bool) == Some(true) {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({
                "error": "worktree creation is not implemented on this server yet — \
                          uncheck 'Use worktree' to create a normal worker"
            })),
        )
            .into_response();
    }
    let path = amux_home().join("sessions").join(format!("{name}.env"));
    if path.exists() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("session '{name}' already exists")})),
        )
            .into_response();
    }
    let dir = s("dir");
    if !dir.is_empty() && !std::path::Path::new(&dir).is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("working directory '{dir}' does not exist")})),
        )
            .into_response();
    }
    let provider = {
        let p = s("provider");
        if p.is_empty() { "claude".to_string() } else { p }
    };
    // The configured default model, the same value the create dialog shows —
    // read from the same place the bootstrap injection reads it, so the
    // dialog and the file it produces cannot disagree.
    let model = {
        let m = s("model");
        if m.is_empty() {
            crate::api::settings::get_default_model(&amux_home())
        } else {
            m
        }
    };
    let mut pairs: Vec<(&str, String)> = vec![("CC_DIR", dir.clone())];
    let creator = s("creator");
    if !creator.is_empty() {
        pairs.push(("CC_CREATOR", creator));
    }
    if provider != "claude" {
        pairs.push(("CC_PROVIDER", provider.clone()));
    }
    if !model.is_empty() {
        pairs.push(("CC_FLAGS", format!("--model {model}")));
    }
    let tags = s("tags");
    if !tags.is_empty() {
        pairs.push(("CC_TAGS", tags));
    }
    let desc = s("desc");
    if !desc.is_empty() {
        pairs.push(("CC_DESC", desc));
    }
    if let Err(e) = write_env_file(&path, &pairs) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("could not write session env: {e}")})),
        )
            .into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "name": name,
            "dir": dir,
            "provider": provider,
            "running": false,
            "archived": false,
        })),
    )
        .into_response()
}

/// The PYTHON fleet's sessions, from the same sources the Python server
/// reads: ~/.amux/sessions/*.env registry + live tmux state. Read-only —
/// the Rust server OBSERVES the Python fleet during coexistence; managing
/// it stays Python's job until cutover. Without this the dashboard on the
/// Rust port says "no workers yet" while 60+ real sessions run (Ethan's
/// first verification finding).
/// Sessions quarantined via blocked-sessions.txt — the Python "archived"
/// flag's source of truth (CC_BLOCKED_SESSIONS, amux-server.py:65).
fn blocked_names(home: &std::path::Path) -> std::collections::BTreeSet<String> {
    std::fs::read_to_string(home.join("blocked-sessions.txt"))
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Test-only fleet suppression. The handler reads `amux_home()` + live tmux at
/// CALL time, so a unit test on a temp DB still merges the machine's real
/// fleet — `legacy_sessions_route_serves_workers…` failed with 117 rows on a
/// box running 116 sessions, and broke every full-suite run (2026-08-09, two
/// lanes hit it). Named deviation: the root fix is capturing home in AppState
/// at startup instead of re-reading env per request (carded); until then this
/// is the only race-free way to keep the unit test's verdict machine-independent.
#[cfg(test)]
pub(crate) static SUPPRESS_FLEET_FOR_TEST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn python_fleet_sessions(signals: &FleetSignals) -> Vec<serde_json::Value> {
    #[cfg(test)]
    if SUPPRESS_FLEET_FOR_TEST.load(std::sync::atomic::Ordering::Relaxed) {
        return vec![];
    }
    let home = amux_home();
    let sessions_dir = home.join("sessions");
    let blocked = blocked_names(&home);
    let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
        return vec![];
    };
    let mut out = vec![];
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("env") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let env = crate::config::parse_env_file(&path);
        let tmux = format!("amux-{name}");
        let is_running = signals.agent_running(&tmux);
        // CC_ARCHIVED=1 is Python's session-archive marker (amux-server.py
        // :20346) — blocked-sessions.txt is QUARANTINE, a different thing;
        // conflating them reported 0 archived against a fleet with dozens.
        let archived = env.get("CC_ARCHIVED").map(|v| v == "1").unwrap_or(false)
            || blocked.contains(&name);
        let flags = env.get("CC_FLAGS").cloned().unwrap_or_default();
        let backend = env
            .get("CC_BACKEND")
            .map(|b| b.trim().to_lowercase())
            .filter(|b| b == "herdr")
            .unwrap_or_else(|| "tmux".into());
        // Python's session_created is the TMUX session's creation time
        // (tinfo["created"], 0 when not running) — not the env file's mtime.
        let session_created = signals.created.get(&tmux).copied().unwrap_or(0);
        // Python's last_activity is meta.last_send falling back to
        // meta.last_started (py:20207-20211) — DELIBERATELY not tmux
        // activity, which updates every snapshot tick and made every lane
        // look equally busy.
        let meta = load_meta(&name);
        let last_activity = {
            let send = meta["last_send"].as_i64().unwrap_or(0);
            if send != 0 { send } else { meta["last_started"].as_i64().unwrap_or(0) }
        };
        let status = signals.derive_status(&name, is_running);
        out.push(json!({
            "archived": archived,
            // The lightning button's state derives from THIS field in the
            // SPA (isYolo checks flags for the provider's skip-permissions
            // flag) — a card without flags renders the wrong YOLO badge
            // (Ethan: "the lightning button isn't correct").
            "flags": flags,
            "creator": env.get("CC_CREATOR").cloned().unwrap_or_default(),
            "backend": backend,
            "auto_continue": env.get("CC_AUTO_CONTINUE").map(|v| v == "1").unwrap_or(false),
            "worktree": env.get("CC_WORKTREE").cloned().unwrap_or_default(),
            "worktree_repo": env.get("CC_WORKTREE_REPO").cloned().unwrap_or_default(),
            "mcp": env.get("CC_MCP").cloned().unwrap_or_default(),
            "session_created": session_created,
            "last_activity": last_activity,
            // Scanner-internal state the Python server holds in memory with
            // no durable trace (rate/credit limits, API errors, the model
            // detector) stays a correct-TYPED honest empty (Invariant 20:
            // never invent). `status` is no longer in that set — it derives
            // above from stores the Python scanner itself persists.
            "active_model": "",
            "api_error": false,
            "api_error_code": "",
            "api_error_count": 0,
            // COMPUTED, NOT HARDCODED (AMUX-2820). These were literal `false`
            // and `0`, with a comment calling them "a correct-TYPED honest
            // empty (Invariant 20: never invent)". That was right at cutover
            // and became a lie by omission the moment nothing filled them:
            // `false` and "not computed" are byte-identical over JSON, so every
            // consumer read a lane parked on Claude Code's rate-limit menu as
            // HEALTHY. mvs-infra sat there with two of Ethan's messages queued
            // behind it and /api/sessions reported status=idle,
            // credit_limited=false the whole time. Nothing downstream — not the
            // log sweep, not autofix, not the invariants monitor — could see a
            // condition its own field says is absent (ethos rule 4).
            //
            // The writer is the rate-limit detector in session_verbs, which
            // stamps meta when it sees the menu and clears it when it answers.
            // Read from meta because THIS LOOP ALREADY LOADS IT — computing it
            // here from a pane capture would cost ~113 tmux calls per request.
            "credit_limited": meta["rate_limited_since"].as_i64().unwrap_or(0) > 0,
            "credit_limit_model": meta["rate_limited_model"].as_str().unwrap_or(""),
            "credit_limited_since": meta["rate_limited_since"].as_i64().unwrap_or(0),
            "rate_limit_banner": meta["rate_limited_since"].as_i64().unwrap_or(0) > 0,
            "rate_limit_weekly": meta["rate_limited_weekly"].as_bool().unwrap_or(false),
            "rate_limited_until": meta["rate_limited_until"].as_i64().unwrap_or(0),
            "last_human_ts": 0,
            "waiting_since": 0,
            "self_report": serde_json::Value::Null,
            // Filled from the shared steering_queue table in build_array —
            // Python's card shape (py:20373), entries {id,text,queued_at,guard}.
            "steering": [],
            "tokens": {"input": 0, "output": 0, "total": 0},
            "preview_lines": [],
            "task_source": "",
            "task_time": 0,
            "task_updated": 0,
            "task_board_id": "",
            "task_board_age": 0,
            "sched_on": 0,
            "sched_off": 0,
            "name": name,
            "status": status,
            "running": is_running,
            "provider": env.get("CC_PROVIDER").cloned().unwrap_or_else(|| "claude".into()),
            "model": env.get("CC_MODEL").cloned().unwrap_or_default(),
            "dir": env.get("CC_DIR").cloned().unwrap_or_default(),
            "preview": "",
            "task_name": "",
            "desc": env.get("CC_DESC").cloned().unwrap_or_default(),
            // TRIMMED, matching Python's t.strip(): CC_TAGS="mvs, gtm"
            // otherwise yields " gtm" beside "gtm" — TWO gtm groups in the
            // UI (Ethan's finding).
            "tags": env.get("CC_TAGS").map(|t| t.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>()).unwrap_or_default(),
            "pinned": env.get("CC_PINNED").map(|v| v == "1").unwrap_or(false),
            "steering_queue": [],
            "managed_by": "python",
        }));
    }
    out
}

/// pub(crate): session_verbs' bare GET /api/sessions/{name} serves ONE
/// record from the SAME array (py:74892 — the natural URL answers the
/// natural shape).
pub(crate) fn build_array(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut signals = FleetSignals::load(conn);
    // Before any status is derived: the pane is the only signal that can
    // contradict a self-report, and a report that nothing can contradict is
    // what shipped a working lane as `idle` for 1076s (AMUX-2646).
    signals.capture_panes();
    let signals = signals;
    let mut stmt = conn.prepare(
        "SELECT w.display_name, w.state, w.provider, w.model, w.cwd,
                (SELECT COUNT(*) FROM _amux_sessions s
                 WHERE s.worker_id = w.id AND s.ended_at IS NULL) AS live
         FROM _amux_workers w
         WHERE json_extract(w.state, '$.deleted_at') IS NULL
         ORDER BY w.display_name",
    )?;
    let rows = stmt.query_map([], |r| {
        let name: String = r.get(0)?;
        let state_json: String = r.get(1)?;
        let provider: String = r.get(2)?;
        let model: Option<String> = r.get(3)?;
        let cwd: String = r.get(4)?;
        let live: i64 = r.get(5)?;
        Ok(json!({
            // The Python list's load-bearing fields; ones the Rust side
            // cannot honestly fill yet are present-and-empty, NOT omitted —
            // the SPA indexes into them.
            "name": name,
            "status": python_status(&state_json),
            "running": live > 0,
            "provider": provider,
            "model": model.unwrap_or_default(),
            "dir": cwd,
            "preview": "",
            "preview_lines": [],
            "task_name": "",
            "task_source": "",
            "task_board_id": "",
            "task_updated": 0,
            "task_board_age": 0,
            "last_activity": 0,
            "pinned": false,
            "desc": "",
            "tags": [],
            "steering_queue": [],
        }))
    })?;
    let mut out: Vec<serde_json::Value> = rows.collect::<Result<_, _>>()?;
    // The Python fleet rides alongside Rust-managed workers, deduped by
    // name (a name registered in BOTH belongs to the Rust row — it carries
    // real state).
    let rust_names: std::collections::BTreeSet<String> = out
        .iter()
        .filter_map(|v| v["name"].as_str().map(|s| s.to_lowercase()))
        .collect();
    for s in python_fleet_sessions(&signals) {
        if let Some(n) = s["name"].as_str() {
            if !rust_names.contains(&n.to_lowercase()) {
                out.push(s);
            }
        }
    }
    // Board linkage per card, Python's exact query + precedence
    // (py:20187-20197, 20348-20365): ORDER BY updated ASC with dict
    // overwrite so the NEWEST-touched doing card wins (the 2026-07-22
    // wrong-task bug), then board-if-fresh(24h) -> meta task_summary ->
    // stale board title -> CC_DESC.
    {
        let mut stmt = conn.prepare(
            "SELECT session, id, title, COALESCE(updated, 0) FROM issues
             WHERE status = 'doing' AND deleted IS NULL AND session IS NOT NULL
             ORDER BY updated ASC",
        )?;
        let mut doing: BTreeMap<String, (String, String, i64)> = BTreeMap::new();
        for row in stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })? {
            let (sess, id, title, updated) = row?;
            doing.insert(sess, (id, title, updated));
        }
        let now = signals.now as i64;
        for v in out.iter_mut() {
            let Some(name) = v["name"].as_str().map(String::from) else {
                continue;
            };
            let board = doing.get(&name);
            let board_updated = board.map(|(_, _, u)| *u).unwrap_or(0);
            let board_fresh = board.is_some() && now - board_updated <= 86400;
            let meta = load_meta(&name);
            let summary = meta["task_summary"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let desc = v["desc"].as_str().unwrap_or("").to_string();
            let (tname, tsrc) = if board_fresh {
                (board.map(|(_, t, _)| t.clone()).unwrap_or_default(), "board")
            } else if !summary.is_empty() {
                (summary, "summary")
            } else if let Some((_, t, _)) = board {
                (t.clone(), "board")
            } else {
                (desc, "desc")
            };
            v["task_name"] = json!(tname);
            v["task_source"] = json!(tsrc);
            v["task_board_id"] =
                json!(if tsrc == "board" { board.map(|(i, _, _)| i.clone()).unwrap_or_default() } else { String::new() });
            // A summary-sourced task now carries its own stamp (AMUX-2676);
            // it is 0 only for tasks written before that existed, and 0 still
            // means "unknown" rather than "just now" — the client must not
            // render an age it does not have.
            v["task_updated"] = json!(match tsrc {
                "board" => board_updated,
                "summary" => meta["task_summary_ts"].as_i64().unwrap_or(0),
                _ => 0,
            });
            v["task_board_age"] = json!(
                if board.is_some() && board_updated != 0 && !board_fresh {
                    (now - board_updated).max(0)
                } else {
                    0
                }
            );
        }
    }

    // Schedule counts per session — Python's exact aggregation
    // (amux-server.py:20179).
    {
        let mut stmt = conn.prepare(
            "SELECT session, SUM(CASE WHEN enabled=1 THEN 1 ELSE 0 END) o,
                    SUM(CASE WHEN enabled=1 THEN 0 ELSE 1 END) f
             FROM schedules
             WHERE deleted IS NULL AND session IS NOT NULL AND session != ''
             GROUP BY session",
        )?;
        let sched: std::collections::BTreeMap<String, (i64, i64)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))?
            .flatten()
            .collect();
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some((on, off)) = sched.get(name) {
                    v["sched_on"] = json!(on);
                    v["sched_off"] = json!(off);
                }
            }
        }
    }

    // steering: Python's card carries the session's queued steering entries
    // (py:20373, `_steering_queue.get(name, [])`) — and that queue is
    // persisted in the shared steering_queue TABLE (INSERT on enqueue,
    // DELETE on delivery, py:8632/8796), so the durable store IS the
    // in-memory queue's mirror. Entry shape matches Python's hydrate
    // (py:11873): {id, text, queued_at, guard} with guard "" for NULL.
    {
        let mut steering: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id, session, text, queued_at, COALESCE(guard,'') \
             FROM steering_queue ORDER BY queued_at ASC",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            }) {
                for (id, session, text, queued_at, guard) in rows.flatten() {
                    steering.entry(session).or_default().push(json!({
                        "id": id, "text": text, "queued_at": queued_at, "guard": guard,
                    }));
                }
            }
        }
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some(q) = steering.get(name) {
                    v["steering"] = json!(q);
                }
            }
        }
    }

    // self_report from the SHARED persisted store (prefs key
    // 'session_reports', amux-server.py:3943) — the same bytes Python
    // hydrates at boot, not its memory. state/ts/source -> Python's
    // {state, age_s, source} card shape (py:20429).
    if signals.reports.is_object() {
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some(rep) = signals.reports.get(name) {
                    // ts is time.time() — a FLOAT; as_i64() read it as 0 and
                    // age_s came out as the whole epoch (found 2026-08-09).
                    let ts = rep["ts"].as_f64().unwrap_or(0.0);
                    v["self_report"] = json!({
                        "state": rep["state"].as_str().unwrap_or(""),
                        "age_s": ((signals.now - ts).max(0.0)) as i64,
                        "source": rep["source"].as_str().unwrap_or(""),
                    });
                    // AMUX-2676: a REPORTED model/token count replaces the
                    // honest-empty above. Still never invented — the empty
                    // stays empty unless the harness itself said otherwise,
                    // which is the whole point of preferring the report
                    // endpoint over a scraper.
                    if let Some(m) = rep["model"].as_str().filter(|m| !m.is_empty()) {
                        v["active_model"] = json!(m);
                    }
                    if rep["tokens"].is_object() {
                        v["tokens"] = rep["tokens"].clone();
                    }
                }
            }
        }
    }

    // branch: bounded parallel git lookups, deduped by directory (many
    // sessions share a checkout — one git call per DISTINCT dir).
    {
        let dirs: std::collections::BTreeSet<String> = out
            .iter()
            .filter_map(|v| v["dir"].as_str())
            .filter(|d| !d.is_empty())
            .map(String::from)
            .collect();
        let mut branches: std::collections::BTreeMap<String, String> = Default::default();
        let dir_list: Vec<String> = dirs.into_iter().collect();
        for chunk in dir_list.chunks(12) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|d| {
                    let d = d.clone();
                    std::thread::spawn(move || {
                        let out = std::process::Command::new("git")
                            .args(["-C", &d, "rev-parse", "--abbrev-ref", "HEAD"])
                            .output()
                            .ok()?;
                        out.status.success().then(|| {
                            (d, String::from_utf8_lossy(&out.stdout).trim().to_string())
                        })
                    })
                })
                .collect();
            for h in handles {
                if let Ok(Some((d, b))) = h.join() {
                    branches.insert(d, b);
                }
            }
        }
        for v in out.iter_mut() {
            let b = v["dir"].as_str().and_then(|d| branches.get(d)).cloned().unwrap_or_default();
            v["branch"] = json!(b);
        }
    }

    // Previews: RUNNING sessions get a bounded parallel tmux capture (30
    // lines like Python's batch, py:20137); STOPPED sessions get the saved
    // log tail (py:20218-20223). Both feed Python's preview pair: scalar +
    // the preview_lines ARRAY the SPA maps over (AMUX-2588).
    {
        let names: Vec<(String, bool)> = out
            .iter()
            .filter_map(|v| {
                let n = v["name"].as_str()?.to_string();
                let running = v["running"].as_bool().unwrap_or(false);
                Some((n, running))
            })
            .collect();
        // Seed from the status probe's captures — same command, same 30 lines.
        // Capturing them twice in one request would be two READS of a live
        // pane 20ms apart, i.e. two different frames, and the card would then
        // show a preview from a frame the status was not derived from.
        let mut raws: std::collections::BTreeMap<String, String> = signals
            .panes
            .iter()
            .filter(|(_, raw)| !raw.trim().is_empty())
            .map(|(n, raw)| (n.clone(), raw.clone()))
            .collect();
        let names: Vec<(String, bool)> =
            names.into_iter().filter(|(n, _)| !raws.contains_key(n)).collect();
        for chunk in names.chunks(12) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(name, running)| {
                    let n = name.clone();
                    let running = *running;
                    std::thread::spawn(move || {
                        if running {
                            // Via the L2 helper, not an inline format!: this was
                            // the ONE tmux target in the crate spelled by hand.
                            // It happened to be exact, but `tmux_target_audit`
                            // cannot tell a correct hand-spelling from the
                            // prefix-matching kind, and a rule with an
                            // exception is a rule nobody can enforce.
                            let pt = pane_target(&format!("amux-{n}"));
                            let out = std::process::Command::new("tmux")
                                .args(["capture-pane", "-t", &pt, "-p", "-e", "-S", "-30"])
                                .output()
                                .ok()?;
                            Some((n, String::from_utf8_lossy(&out.stdout).trim().to_string()))
                        } else {
                            let raw = stopped_session_raw(&n);
                            (!raw.is_empty()).then_some((n, raw))
                        }
                    })
                })
                .collect();
            for h in handles {
                if let Ok(Some((n, p))) = h.join() {
                    raws.insert(n, p);
                }
            }
        }
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some(raw) = raws.get(name) {
                    let (preview, lines) = preview_of(raw);
                    v["preview"] = json!(preview);
                    v["preview_lines"] = json!(lines);
                }
            }
        }
    }

    // Python's exact sort (py:20456-20457): pinned first, running next,
    // active/waiting before idle/blank, then most-recent human activity.
    let status_rank = |s: &str| -> i64 {
        match s {
            "active" | "waiting" => 0,
            _ => 1,
        }
    };
    out.sort_by(|a, b| {
        let key = |v: &serde_json::Value| {
            (
                !v["pinned"].as_bool().unwrap_or(false),
                !v["running"].as_bool().unwrap_or(false),
                status_rank(v["status"].as_str().unwrap_or("")),
                -v["last_activity"].as_i64().unwrap_or(0),
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_vocabulary_matches_python() {
        assert_eq!(python_status(r#"{"state":"active","turn":null}"#), "active");
        assert_eq!(python_status(r#"{"state":"idle","since":"x"}"#), "idle");
        assert_eq!(python_status(r#"{"state":"rate_limited","reset_at":null}"#), "rate-limited");
        assert_eq!(python_status(r#"{"state":"stopped"}"#), "");
    }

    pub(super) fn signals() -> FleetSignals {
        FleetSignals {
            activity: BTreeMap::new(),
            created: BTreeMap::new(),
            running: BTreeSet::new(),
            shell_only: BTreeSet::new(),
            reports: serde_json::Value::Null,
            transitions: BTreeMap::new(),
            started: BTreeMap::new(),
            panes: BTreeMap::new(),
            now: 1_000_000.0,
        }
    }

    #[test]
    fn status_blank_when_not_running() {
        let s = signals();
        assert_eq!(s.derive_status("x", false), "");
    }

    #[test]
    fn status_active_on_recent_activity_idle_otherwise() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), 999_970); // 30s ago
        assert_eq!(s.derive_status("x", true), "active");
        s.activity.insert("amux-x".into(), 999_000); // 1000s ago
        assert_eq!(s.derive_status("x", true), "idle");
    }

    #[test]
    fn status_prefers_persisted_transition_including_waiting() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), 999_000);
        s.transitions.insert("x".into(), ("waiting".into(), 999_900.0));
        assert_eq!(s.derive_status("x", true), "waiting");
    }

    #[test]
    fn stale_active_transition_demotes_to_idle() {
        let mut s = signals();
        // Transition says active, but the pane has been silent 1000s (>120).
        s.activity.insert("amux-x".into(), 999_000);
        s.transitions.insert("x".into(), ("active".into(), 999_100.0));
        assert_eq!(s.derive_status("x", true), "idle");
    }

    #[test]
    fn pre_restart_transition_is_discarded() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), 999_970);
        s.transitions.insert("x".into(), ("waiting".into(), 900.0));
        s.started.insert("x".into(), 999_000.0); // restarted AFTER the event
        assert_eq!(s.derive_status("x", true), "active"); // falls to activity
    }

    #[test]
    fn self_report_overrides_with_asymmetric_freshness() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), 999_970); // scrape would say active
        // A 4h-old idle report STILL wins (idle does not decay, py:20233).
        s.reports = json!({"x": {"state": "idle", "ts": 985_600.0, "source": "stop-hook"}});
        assert_eq!(s.derive_status("x", true), "idle");
        // A 4h-old ACTIVE report licenses nothing (heartbeat lapsed).
        s.reports = json!({"x": {"state": "active", "ts": 985_600.0, "source": "hb"}});
        s.activity.insert("amux-x".into(), 999_000);
        assert_eq!(s.derive_status("x", true), "idle");
        // A fresh waiting report wins over the activity fallback.
        s.reports = json!({"x": {"state": "waiting", "ts": 999_990.0, "source": "hook"}});
        assert_eq!(s.derive_status("x", true), "waiting");
    }

    #[test]
    fn preview_lines_is_a_filtered_array_of_strings() {
        let raw = "\u{1b}[1mDoing the work\u{1b}[0m\n\
                   ⏵⏵ bypass permissions on\n\
                   ══════════════════════\n\
                   ok\n\
                   Implemented the fix in board.rs\n\
                   x\n";
        let (preview, lines) = preview_of(raw);
        // Scalar preview: last intelligible line (skips ⏵⏵, short, low-alnum).
        assert_eq!(preview, "Implemented the fix in board.rs");
        // Array: bars (low alnum ratio), the ⏵⏵ line, and <=2-char lines
        // are dropped; ANSI is stripped from kept lines.
        assert_eq!(lines, vec!["Doing the work", "Implemented the fix in board.rs"]);
    }

    #[test]
    fn preview_skips_status_bar_line() {
        let raw = "Some output text\n\
                   \u{276f}\u{a0}\n\
                   ──────\n\
                   \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle)\n";
        let (preview, _) = preview_of(raw);
        assert_eq!(preview, "Some output text");
    }

    #[test]
    fn preview_lines_falls_back_to_raw_tail_when_nothing_intelligible() {
        // Every line >3 chars with alnum ratio < 0.3 -> nothing intelligible.
        let raw = "════\n────\n╭──╮\n";
        let (_, lines) = preview_of(raw);
        // Fallback keeps the stripped non-empty tail lines (py:20314-20316).
        assert_eq!(lines, vec!["════", "────", "╭──╮"]);
    }

    #[test]
    fn preview_truncates_at_python_lengths() {
        let long = "a".repeat(300);
        let (preview, lines) = preview_of(&long);
        assert_eq!(preview.chars().count(), 120);
        assert_eq!(lines[0].chars().count(), 200);
    }
}

// ---------------------------------------------------------------------------
// AMUX-2646 — "it is running but says idle".
//
// The frames below are VERBATIM captures of the live fleet on 2026-08-09,
// not constructed ones. That matters: the convenient fixture is convenient
// precisely because it lacks the property that made the incident. Two of
// these were built by hand first and were wrong in ways that would have made
// the suite pass against the bug —
//
//   * a "generating" frame with `esc to interrupt` on the bar. The lane that
//     was actually mislabelled (`amux-rust`) had NO such bar; its only mark
//     was a live spinner. A suite built on the first frame alone would have
//     been green against the specimen it exists for.
//   * an "idle with background agents" frame carrying `esc to interrupt` on
//     the bar — the shape of the theory `pane_bar_says_generating` records
//     itself REJECTING ("empty ❯ + esc to interrupt = idle with background
//     agents"). Whether that frame exists decides whether the override below
//     is safe at all, and it had never been measured either way. It was here:
//     across four live lanes, this Claude Code build prints `esc to interrupt`
//     only while the MAIN turn is generating — an idle lane with two agents
//     shows `⏵⏵ bypass permissions on (shift+tab to cycle) · ← 2 agents` and
//     a completed-turn marker (`✻ Churned for 2m 57s`). So the bar is a sound
//     work signal, and the rejection in that function was right for a reason
//     nobody had confirmed. IDLE_WITH_AGENTS below is the real frame; if a
//     future Claude Code starts painting `esc to interrupt` for background
//     agents, that test goes red and this override needs rethinking, which is
//     the whole point of keeping the frame rather than a paraphrase of it.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod status_truth {
    use super::tests::signals;
    use super::*;

    /// Live `amux`, mid-turn: spinner AND `esc to interrupt` on the bar.
    const WORKING_BAR: &str = "\
2436    const _active = document.activeElement;
\u{273b} Doing\u{2026} (3m 56s \u{b7} \u{2193} 6.8k tokens)
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
\u{276f}
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
  \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle) \u{b7} esc to interrupt \u{b7} \u{2190} 2 agents";

    /// Live `amux-rust`, mid-turn — THE SPECIMEN. Nothing on the status bar;
    /// the spinner is the only evidence. This is the lane that showed `idle`
    /// on its card for 1076s while it was demonstrably working.
    const WORKING_SPINNER_ONLY: &str = "\
\u{273b} Nesting\u{2026} (4m 24s \u{b7} \u{2193} 6.4k tokens)
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
\u{276f} [05:08 PM] this worker is doing work but there isnt anything in inprogress
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
  \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle)";

    /// Live `amux-frustrations`: genuinely idle, two BACKGROUND agents still
    /// running. The completed-turn marker (`for 2m 57s`), an empty composer,
    /// and no `esc to interrupt`. This is the frame that must NOT be read as
    /// work, or every finished lane with agents flips to active forever.
    const IDLE_WITH_AGENTS: &str = "\
  Left at done, not verified \u{2014} live behaviour confirmed.
\u{273b} Churned for 2m 57s
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
\u{276f}
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
  \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle) \u{b7} \u{2190} 2 agents";

    /// Live `uitest-a`: the agent exited, tmux session still up.
    const SHELL_PROMPT: &str = "\
tmp$ unset ANTHROPIC_API_KEY
tmp$ claude --model claude-opus-4-6 --dangerously-skip-permissions
Resume this session with:
claude --resume \"uitest-a\"
tmp$";

    /// A permission selector — waiting on a human, not working.
    const WAITING_SELECTOR: &str = "\
Do you want to proceed?
\u{276f} 1. Yes
  2. No, and tell Claude what to do differently (esc)
  \u{23f5}\u{23f5} bypass permissions on (shift+tab to cycle)";

    /// The usage-limit menu (D2). Also a human decision, never work.
    const RATE_LIMIT_MENU: &str = "\
Claude usage limit reached. Your limit will reset at 3pm.
\u{276f} 1. Wait and continue
  2. Switch to a different model";

    /// A herdr lane MID-TURN. herdr refuses a history read while it is
    /// working, so the capture is empty BY DESIGN — the one frame where
    /// "no markers" must not mean "idle".
    const HERDR_MID_TURN: &str = "";

    /// One row of the truth table.
    struct Case {
        what: &'static str,
        /// (state, age_s, source)
        report: Option<(&'static str, f64, &'static str)>,
        /// (state, age_s)
        transition: Option<(&'static str, f64)>,
        pane: Option<&'static str>,
        /// How long ago the pane last painted.
        activity_age_s: f64,
        running: bool,
        expect: &'static str,
    }

    fn run(c: &Case) -> String {
        let mut s = signals();
        s.activity.insert("x".into(), 0); // never matched: keys are `amux-<n>`
        s.activity.insert("amux-x".into(), (s.now - c.activity_age_s) as i64);
        if c.running {
            s.running.insert("amux-x".into());
        }
        if let Some((st, age, src)) = c.report {
            s.reports = json!({"x": {"state": st, "ts": s.now - age, "source": src}});
        }
        if let Some((st, age)) = c.transition {
            s.transitions.insert("x".into(), (st.into(), s.now - age));
        }
        if let Some(p) = c.pane {
            s.panes.insert("x".into(), p.into());
        }
        s.derive_status("x", c.running)
    }

    /// THE TABLE. Every cell is a (report, age, source, pane, activity,
    /// running) combination with the status it must produce.
    #[test]
    fn status_truth_table() {
        let cases = [
            // ---- the bug, in both of its live shapes -------------------
            Case {
                what: "STALE idle report + pane mid-turn (bar) = THE BUG",
                report: Some(("idle", 1076.0, "stop-hook-test")),
                transition: None,
                pane: Some(WORKING_BAR),
                activity_age_s: 1.0,
                running: true,
                expect: "active",
            },
            Case {
                what: "STALE idle report + pane mid-turn (spinner only) = THE SPECIMEN",
                report: Some(("idle", 1076.0, "stop-hook-test")),
                transition: None,
                pane: Some(WORKING_SPINNER_ONLY),
                activity_age_s: 1.0,
                running: true,
                expect: "active",
            },
            Case {
                what: "a DAY-old idle report loses to a live pane just the same",
                report: Some(("idle", 80_000.0, "stop-hook")),
                transition: None,
                pane: Some(WORKING_BAR),
                activity_age_s: 2.0,
                running: true,
                expect: "active",
            },
            // ---- the grace window: a fresh report is still the authority
            Case {
                what: "FRESH idle report wins over the pane (report/repaint race)",
                report: Some(("idle", 3.0, "stop-hook")),
                transition: None,
                pane: Some(WORKING_BAR),
                activity_age_s: 1.0,
                running: true,
                expect: "idle",
            },
            Case {
                what: "fresh idle report + quiet pane: plain idle",
                report: Some(("idle", 5.0, "stop-hook")),
                transition: None,
                pane: Some(IDLE_WITH_AGENTS),
                activity_age_s: 1.0,
                running: true,
                expect: "idle",
            },
            // ---- silence is NOT contradiction --------------------------
            Case {
                what: "stale idle + a parked lane that has not painted: stays idle",
                report: Some(("idle", 9_000.0, "stop-hook")),
                transition: None,
                // The pane still holds a mid-turn frame in scrollback, but
                // nothing has painted for an hour: not evidence.
                pane: Some(WORKING_BAR),
                activity_age_s: 3_600.0,
                running: true,
                expect: "idle",
            },
            Case {
                what: "idle lane WITH BACKGROUND AGENTS is idle, not active",
                report: Some(("idle", 700.0, "stop-hook")),
                transition: None,
                pane: Some(IDLE_WITH_AGENTS),
                activity_age_s: 2.0,
                running: true,
                expect: "idle",
            },
            // ---- no report at all (hookless lane, dropped POST) --------
            Case {
                what: "no report + working pane",
                report: None,
                transition: None,
                pane: Some(WORKING_SPINNER_ONLY),
                activity_age_s: 1.0,
                running: true,
                expect: "active",
            },
            Case {
                what: "no report + shell prompt (agent exited)",
                report: None,
                transition: None,
                pane: Some(SHELL_PROMPT),
                activity_age_s: 1.0,
                running: true,
                expect: "idle",
            },
            Case {
                what: "no report + selector = waiting on a human",
                report: None,
                transition: None,
                pane: Some(WAITING_SELECTOR),
                activity_age_s: 1.0,
                running: true,
                expect: "waiting",
            },
            Case {
                what: "no report + usage-limit menu = waiting (never invented as active)",
                report: None,
                transition: None,
                pane: Some(RATE_LIMIT_MENU),
                activity_age_s: 1.0,
                running: true,
                expect: "waiting",
            },
            // ---- active reports ---------------------------------------
            Case {
                what: "fresh active report + dead/unreadable pane: believed",
                report: Some(("active", 10.0, "tool-hook")),
                transition: None,
                pane: Some(HERDR_MID_TURN),
                activity_age_s: 1.0,
                running: true,
                expect: "active",
            },
            Case {
                what: "STALE active report (past the heartbeat) never overrides",
                report: Some(("active", 4_000.0, "tool-hook")),
                transition: None,
                pane: Some(IDLE_WITH_AGENTS),
                activity_age_s: 2.0,
                running: true,
                expect: "idle",
            },
            // ---- herdr: an empty capture is not evidence of anything ---
            Case {
                what: "herdr lane, empty capture, painting: NOT idle",
                report: None,
                transition: None,
                pane: Some(HERDR_MID_TURN),
                activity_age_s: 2.0,
                running: true,
                expect: "active",
            },
            Case {
                what: "herdr lane, empty capture, silent for an hour: idle",
                report: None,
                transition: None,
                pane: Some(HERDR_MID_TURN),
                activity_age_s: 3_600.0,
                running: true,
                expect: "idle",
            },
            // ---- transitions and liveness ------------------------------
            Case {
                what: "waiting transition survives (no report, no pane)",
                report: None,
                transition: Some(("waiting", 100.0)),
                pane: None,
                activity_age_s: 3_600.0,
                running: true,
                expect: "waiting",
            },
            Case {
                what: "stale active transition demotes when the pane is silent",
                report: None,
                transition: Some(("active", 900.0)),
                pane: None,
                activity_age_s: 1_000.0,
                running: true,
                expect: "idle",
            },
            Case {
                what: "not running is blank, whatever anything else says",
                report: Some(("active", 1.0, "tool-hook")),
                transition: None,
                pane: Some(WORKING_BAR),
                activity_age_s: 1.0,
                running: false,
                expect: "",
            },
        ];
        let mut failed = vec![];
        for c in &cases {
            let got = run(c);
            if got != c.expect {
                failed.push(format!("  {}\n     want {:?}, got {:?}", c.what, c.expect, got));
            }
        }
        assert!(failed.is_empty(), "status truth table:\n{}", failed.join("\n"));
    }

    /// THE PROPERTY, over the full product of the table's inputs: a lane whose
    /// pane is unambiguously mid-turn is never reported `idle` — unless an
    /// idle report younger than the contradiction window is standing behind
    /// it, which is the one deliberate exception (the report is the D1
    /// authority and this is where the report/repaint race lives).
    ///
    /// Exhaustive rather than random: the input space is small enough to
    /// enumerate, and an enumerated space cannot get lucky.
    #[test]
    fn no_input_combination_reports_idle_over_a_working_pane() {
        let states = ["idle", "active", "waiting", "error", "bogus"];
        let ages = [0.0, 1.0, 59.0, 61.0, 121.0, 1_076.0, 1_801.0, 86_401.0];
        let sources = ["stop-hook", "tool-hook", "prompt-hook", "stop-hook-test", ""];
        let working_panes = [WORKING_BAR, WORKING_SPINNER_ONLY];
        let act_ages = [0.0, 1.0, 30.0, 59.0];
        let transitions = [None, Some(("idle", 10.0)), Some(("active", 900.0)), Some(("waiting", 5.0))];
        let mut checked = 0usize;
        for pane in working_panes {
            for act in act_ages {
                for tr in transitions {
                    for st in states {
                        for age in ages {
                            for src in sources {
                                let c = Case {
                                    what: "property",
                                    report: Some((st, age, src)),
                                    transition: tr,
                                    pane: Some(pane),
                                    activity_age_s: act,
                                    running: true,
                                    expect: "",
                                };
                                let got = run(&c);
                                checked += 1;
                                let grace = st == "idle" && age <= 60.0;
                                assert!(
                                    got != "idle" || grace,
                                    "idle over a working pane: report=({st},{age}s,{src}) \
                                     transition={tr:?} activity_age={act}s"
                                );
                            }
                        }
                    }
                }
            }
        }
        // The enumeration must have RUN. A property test over an empty product
        // passes vacuously and looks identical to one that proved something.
        assert_eq!(checked, 2 * 4 * 4 * 5 * 8 * 5);
        // And the exception must be REACHABLE, or "unless grace" is dead prose
        // rather than a documented carve-out.
        assert_eq!(
            run(&Case {
                what: "grace is reachable",
                report: Some(("idle", 5.0, "stop-hook")),
                transition: None,
                pane: Some(WORKING_BAR),
                activity_age_s: 0.0,
                running: true,
                expect: "idle",
            }),
            "idle"
        );
    }

    /// The two detectors this composes must actually discriminate the frames.
    /// If `pane_says_working` returned true for everything, the table above
    /// would still pass on its bug rows while quietly breaking every idle row
    /// — so assert the evidence function's verdict directly, per frame.
    #[test]
    fn evidence_discriminates_between_the_live_frames() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), s.now as i64);
        let verdict = |s: &mut FleetSignals, raw: &str| {
            s.panes.insert("x".into(), raw.into());
            s.pane_says_working("x")
        };
        assert!(verdict(&mut s, WORKING_BAR), "bar `esc to interrupt` is work");
        assert!(verdict(&mut s, WORKING_SPINNER_ONLY), "a live spinner is work");
        assert!(!verdict(&mut s, IDLE_WITH_AGENTS), "background agents are NOT the main turn");
        assert!(!verdict(&mut s, SHELL_PROMPT), "a shell is not work");
        assert!(!verdict(&mut s, WAITING_SELECTOR), "waiting on a human is not work");
        assert!(!verdict(&mut s, RATE_LIMIT_MENU), "a usage-limit menu is not work");
        assert!(!verdict(&mut s, HERDR_MID_TURN), "an empty capture proves nothing");
    }

    /// Evidence must be admissible only while it is FRESH — this is the half
    /// that keeps `idle survives silence` true, and the half a reader is most
    /// likely to delete as redundant.
    #[test]
    fn stale_evidence_is_inadmissible_however_loud_it_is() {
        let mut s = signals();
        s.panes.insert("x".into(), WORKING_BAR.into());
        s.activity.insert("amux-x".into(), (s.now - 61.0) as i64);
        assert!(!s.pane_says_working("x"), "a pane that has not painted in 61s is not evidence");
        s.activity.insert("amux-x".into(), (s.now - 59.0) as i64);
        assert!(s.pane_says_working("x"), "…and one that painted 59s ago is");
    }

    /// The capture predicate and the belief predicate are the same predicate.
    /// If they drift, the board (which captures a few panes) and the session
    /// list (which has every running pane in hand) derive different statuses
    /// for the same lane, and the user sees a card that contradicts itself.
    #[test]
    fn a_pane_the_probe_would_not_have_taken_is_not_believed() {
        let mut s = signals();
        s.activity.insert("amux-x".into(), (s.now - 6_000.0) as i64);
        assert!(!s.pane_probe_candidate("x"));
        // A caller stuffs the map anyway (a superset capture, or a test).
        s.panes.insert("x".into(), WORKING_BAR.into());
        assert!(!s.pane_says_working("x"), "belief must re-apply the capture predicate");
        assert_eq!(s.derive_status("x", true), "idle");
    }

    /// THE LIVE-FLEET CONSISTENCY CHECK, read-only, on demand:
    ///
    /// ```text
    /// CARGO_TARGET_DIR=/tmp/amux-status-target cargo test -p amux-server \
    ///   sessions_legacy::status_truth::live_fleet -- --ignored --nocapture
    /// ```
    ///
    /// `#[ignore]` because it reads the machine's real fleet, so it is not a
    /// CI check — it is the sweep instrument. It opens `~/.amux/amux.db`
    /// READ-ONLY (never the live DB read-write: this is real user data) and
    /// captures panes, which is what `tmux capture-pane -p` already does on
    /// every dashboard poll.
    ///
    /// It exists because the ONLY thing that caught AMUX-2646 was a human
    /// noticing a terminal. This is that human, as a command, in one second.
    #[test]
    #[ignore = "reads the live fleet; run explicitly with --ignored"]
    fn live_fleet_status_matches_pane_truth() {
        let home = std::env::var("HOME").unwrap_or_default();
        let db = format!("{home}/.amux/amux.db");
        let conn = rusqlite::Connection::open_with_flags(
            &db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap_or_else(|e| panic!("live db {db} unreadable: {e}"));
        let mut s = FleetSignals::load(&conn);
        assert!(!s.running.is_empty(), "no tmux fleet visible — probe is broken, not fleet empty");
        s.capture_panes();
        let probed = s.probed_lanes();
        let mut bad = vec![];
        for (name, working) in &probed {
            let status = s.derive_status(name, true);
            let rep = s.reports.get(name).cloned().unwrap_or(json!({}));
            if *working && status == "idle" {
                bad.push(format!(
                    "  {name}: card=idle but the pane is mid-turn \
                     (report={} age={:.0}s source={} origin={})",
                    rep["state"].as_str().unwrap_or("-"),
                    s.now - rep["ts"].as_f64().unwrap_or(s.now),
                    rep["source"].as_str().unwrap_or("-"),
                    rep["origin"].as_str().unwrap_or("-"),
                ));
            }
        }
        // The whole registry, not only the probed lanes: a status histogram is
        // how a REGRESSION in the other direction shows up (everything flipping
        // to active), which a disagreement count of 0 would happily hide.
        let mut hist: BTreeMap<String, usize> = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(amux_home().join("sessions")) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("env") {
                    continue;
                }
                let Some(n) = p.file_stem().and_then(|x| x.to_str()) else { continue };
                let running = s.agent_running(&format!("amux-{n}"));
                let st = s.derive_status(n, running);
                *hist.entry(if st.is_empty() { "<blank>".into() } else { st }).or_default() += 1;
            }
        }
        println!(
            "live fleet: {} tmux sessions, {} painted inside the probe window, \
             {} of those mid-turn, DISAGREEMENTS: {}\n  status histogram: {:?}",
            s.running.len(),
            probed.len(),
            probed.iter().filter(|(_, w)| *w).count(),
            bad.len(),
            hist
        );
        for l in &bad {
            println!("{l}");
        }
        assert!(bad.is_empty(), "card/pane disagreements:\n{}", bad.join("\n"));
    }

    /// tmux's `session_activity` does not move for a DETACHED session, and
    /// every amux lane is detached — so the parser must take the max with
    /// `window_activity` or the fleet's only liveness signal reads as
    /// permanent silence (measured: 60/63 lanes, one of them 34.5h stale).
    #[test]
    fn activity_is_the_max_of_session_and_window() {
        // The REAL line `amux-rust` produced while it was mid-turn, through
        // the REAL parser. `session_activity` had not moved since the session
        // was created 34.5h earlier; `window_activity` was current.
        let line = "amux-rust:1786206640:1786206640:1786330900";
        assert_eq!(
            parse_list_sessions_line(line),
            Some(("amux-rust", Some(1_786_330_900), Some(1_786_206_640))),
            "window activity must win when it is newer — this is the whole fleet's \
             only liveness signal"
        );
        // The other direction still works, and a short line does not panic.
        assert_eq!(
            parse_list_sessions_line("amux-x:200:100:50"),
            Some(("amux-x", Some(200), Some(100)))
        );
        assert_eq!(parse_list_sessions_line("amux-x:200"), Some(("amux-x", Some(200), None)));
        assert_eq!(parse_list_sessions_line(""), None);
    }
}
