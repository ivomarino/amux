//! GET /api/sessions — the PYTHON-SHAPED session list (RR-0075 enabler).
//!
//! The alias layer rewrites legacy PATHS, but the SPA also expects the
//! Python RESPONSE SHAPE: a bare array of `{name, status, preview, ...}`.
//! The modern /api/workers envelope (items/total, display_name, typed
//! state) is right for new clients; this projection is what lets the
//! 44k-line dashboard render workers today, unchanged. It is registered
//! BEFORE the rewrite middleware so it wins over the path alias.

use super::AppState;
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
//   "" :   not running.
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

/// Signals the derivation reads, loaded once per request and shared with the
/// board's `stale` computation (`active_python_sessions`) so the two can
/// never disagree about who is working.
pub struct FleetSignals {
    /// tmux session name (`amux-<n>`) -> `#{session_activity}`.
    pub activity: BTreeMap<String, i64>,
    /// tmux session name -> `#{session_created}`.
    pub created: BTreeMap<String, i64>,
    /// Live tmux session names.
    pub running: BTreeSet<String>,
    /// The persisted self-report store (prefs `session_reports`,
    /// amux-server.py:3943) — the same bytes Python hydrates at boot.
    pub reports: serde_json::Value,
    /// session -> (status, ts) of its latest working/idle/waiting transition.
    pub transitions: BTreeMap<String, (String, f64)>,
    /// session -> ts of its latest `session.started` event.
    pub started: BTreeMap<String, f64>,
    pub now: f64,
}

impl FleetSignals {
    pub fn load(conn: &rusqlite::Connection) -> Self {
        let mut activity = BTreeMap::new();
        let mut created = BTreeMap::new();
        let mut running = BTreeSet::new();
        if let Ok(o) = std::process::Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_activity}\t#{session_created}",
            ])
            .output()
        {
            for l in String::from_utf8_lossy(&o.stdout).lines() {
                let mut it = l.split('\t');
                let (Some(n), a, c) = (it.next(), it.next(), it.next()) else {
                    continue;
                };
                running.insert(n.to_string());
                if let Some(ts) = a.and_then(|x| x.parse().ok()) {
                    activity.insert(n.to_string(), ts);
                }
                if let Some(ts) = c.and_then(|x| x.parse().ok()) {
                    created.insert(n.to_string(), ts);
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
            reports,
            transitions,
            started,
            now: chrono::Utc::now().timestamp() as f64,
        }
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
        let mut status = status
            .unwrap_or_else(|| if self.now - act < 60.0 { "active".into() } else { "idle".into() });
        // self_report override — Python's exact gate (py:20248-20263).
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
            }
        }
        status
    }
}

/// The set of sessions currently `active` — the board's `stale` flag reads
/// this (Python: `_session_prev_status[sess] == "active"`, py:15671-15697).
/// Shares `FleetSignals` with the session list: one derivation, two readers.
pub fn active_python_sessions(conn: &rusqlite::Connection) -> BTreeSet<String> {
    let signals = FleetSignals::load(conn);
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
        let running = signals.running.contains(&format!("amux-{name}"));
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
fn strip_ansi(s: &str) -> String {
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
        .last()
        .map(|l| strip_ansi(&chars_truncate(l, 120)))
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

fn python_fleet_sessions(signals: &FleetSignals) -> Vec<serde_json::Value> {
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
        let is_running = signals.running.contains(&tmux);
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
            "credit_limited": false,
            "credit_limit_model": "",
            "credit_limited_since": 0,
            "rate_limit_banner": false,
            "rate_limit_weekly": false,
            "rate_limited_until": 0,
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

fn build_array(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<serde_json::Value>> {
    let signals = FleetSignals::load(conn);
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
            let summary = load_meta(&name)["task_summary"]
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
            v["task_updated"] = json!(if board.is_some() { board_updated } else { 0 });
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
        let mut raws: std::collections::BTreeMap<String, String> = Default::default();
        for chunk in names.chunks(12) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(name, running)| {
                    let n = name.clone();
                    let running = *running;
                    std::thread::spawn(move || {
                        if running {
                            let out = std::process::Command::new("tmux")
                                .args([
                                    "capture-pane",
                                    "-t",
                                    &format!("=amux-{n}:"),
                                    "-p",
                                    "-e",
                                    "-S",
                                    "-30",
                                ])
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

    fn signals() -> FleetSignals {
        FleetSignals {
            activity: BTreeMap::new(),
            created: BTreeMap::new(),
            running: BTreeSet::new(),
            reports: serde_json::Value::Null,
            transitions: BTreeMap::new(),
            started: BTreeMap::new(),
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
        // Scalar preview: last non-blank raw line, stripped, <=120 chars.
        assert_eq!(preview, "x");
        // Array: bars (low alnum ratio), the ⏵⏵ line, and <=2-char lines
        // are dropped; ANSI is stripped from kept lines.
        assert_eq!(lines, vec!["Doing the work", "Implemented the fix in board.rs"]);
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
