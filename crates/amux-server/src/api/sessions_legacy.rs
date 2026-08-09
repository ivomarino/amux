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
        out.push(json!({
            "archived": archived,
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
    // task_name: the session's current doing card, the board linkage the
    // Python cards carry (one query for the whole list, not N).
    {
        let mut stmt = conn.prepare(
            "SELECT session, title FROM issues
             WHERE deleted IS NULL AND status = 'doing' AND session != ''
             GROUP BY session",
        )?;
        let doing: std::collections::BTreeMap<String, String> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .flatten()
            .collect();
        for v in out.iter_mut() {
            if let Some(name) = v["name"].as_str() {
                if let Some(title) = doing.get(name) {
                    v["task_name"] = json!(title);
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
