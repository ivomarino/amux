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

fn python_fleet_sessions() -> Vec<serde_json::Value> {
    let home = std::env::var("AMUX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
        });
    let sessions_dir = home.join("sessions");
    let running: std::collections::BTreeSet<String> = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let blocked = blocked_names(&home);
    // One tmux call for the whole fleet: name -> activity ts (Python's
    // _tmux_info_map shape, the field every card's last_activity reads).
    let activity: std::collections::BTreeMap<String, i64> = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name} #{session_activity}"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let (n, ts) = l.rsplit_once(' ')?;
                    Some((n.to_string(), ts.parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default();
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
        let is_running = running.contains(&format!("amux-{name}"));
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
        let session_created = path
            .metadata()
            .ok()
            .and_then(|m| m.created().or_else(|_| m.modified()).ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let last_activity = activity
            .get(&format!("amux-{name}"))
            .copied()
            .unwrap_or(0);
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
            // Scanner-internal state the Python server holds in memory —
            // the Rust server does not run that scanner. Correct-TYPED
            // honest empties (Invariant 20: never invent), so the SPA
            // renders identically-shaped cards.
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
            "steering": [],
            "tokens": {"input": 0, "output": 0, "total": 0},
            "preview_lines": 0,
            "task_source": "",
            "task_time": 0,
            "task_updated": 0,
            "task_board_id": "",
            "task_board_age": 0,
            "sched_on": 0,
            "sched_off": 0,
            "name": name,
            // Status detail (active/idle) is the Python scanner's judgment;
            // the honest cells from HERE are running-blank vs stopped-blank
            // plus the running flag the list renders.
            "status": "",
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
            "task_name": "",
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
    for s in python_fleet_sessions() {
        if let Some(n) = s["name"].as_str() {
            if !rust_names.contains(&n.to_lowercase()) {
                out.push(s);
            }
        }
    }
    // Board linkage per card: doing card title + id + updated (Python's
    // task_name/task_board_id/task_updated/task_board_age), one query.
    {
        let mut stmt = conn.prepare(
            "SELECT session, title, id, COALESCE(updated, 0) FROM issues
             WHERE deleted IS NULL AND status = 'doing' AND session != ''
             GROUP BY session",
        )?;
        let doing: std::collections::BTreeMap<String, (String, String, i64)> = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
            })?
            .flatten()
            .collect();
        let now = chrono::Utc::now().timestamp();
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str().map(String::from) {
                if let Some((title, id, updated)) = doing.get(&name) {
                    v["task_name"] = json!(title);
                    v["task_board_id"] = json!(id);
                    v["task_updated"] = json!(updated);
                    v["task_source"] = json!("board");
                    v["task_board_age"] = json!((now - updated).max(0));
                }
            }
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

    // Previews for RUNNING sessions: bounded parallel tmux capture, 15-line
    // ANSI tail like Python's card preview. Bounded (12 at a time) so 49
    // running sessions cannot serialize the request.
    {
        let running: Vec<String> = out
            .iter()
            .filter(|v| v["running"].as_bool().unwrap_or(false))
            .filter_map(|v| v["name"].as_str().map(String::from))
            .collect();
        let mut previews: std::collections::BTreeMap<String, String> = Default::default();
        for chunk in running.chunks(12) {
            let handles: Vec<_> = chunk
                .iter()
                .map(|name| {
                    let n = name.clone();
                    std::thread::spawn(move || {
                        let out = std::process::Command::new("tmux")
                            .args([
                                "capture-pane",
                                "-t",
                                &format!("=amux-{n}:"),
                                "-p",
                                "-e",
                                "-S",
                                "-15",
                            ])
                            .output()
                            .ok()?;
                        Some((n, String::from_utf8_lossy(&out.stdout).trim_end().to_string()))
                    })
                })
                .collect();
            for h in handles {
                if let Ok(Some((n, p))) = h.join() {
                    previews.insert(n, p);
                }
            }
        }
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some(p) = previews.get(name) {
                    v["preview"] = json!(p);
                    v["preview_lines"] = json!(p.lines().count());
                }
            }
        }
    }

    // Running first, then name — the Python list's ordering instinct.
    out.sort_by(|a, b| {
        let ra = a["running"].as_bool().unwrap_or(false);
        let rb = b["running"].as_bool().unwrap_or(false);
        rb.cmp(&ra).then_with(|| {
            a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
        })
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
}
