//! /api/env — declarative environment config (AMUX-2977, Ethan's centerpiece).
//!
//! One YAML that CREATES an amux environment by configuring the existing
//! primitives — this is a loader OVER the primitives (groups, workers,
//! schedulers, board, files), NOT a new primitive, which is the whole ethos:
//! "model your organization as configuration of the eight primitives."
//!
//!   POST /api/env/apply            apply the YAML (idempotent)
//!   POST /api/env/apply?dry_run=1  report what WOULD change, write nothing
//!   GET  /api/env/schema           the accepted shape, as docs
//!
//! Body is YAML (Content-Type text/yaml or application/x-yaml) OR JSON — both
//! parse to the same shape. Idempotent by IDENTITY: a group is its name, a
//! worker is its env file, so applying twice converges instead of duplicating.
//!
//! PHASE 1 (this) covers the org STRUCTURE — `groups` and `workers` — the two
//! primitives everything else hangs off. `schedules`, board `columns` + `gates`,
//! seed `files`, and `global` env are PHASE 2, each an additive stanza the
//! report already accounts for as "not-yet-applied" so nothing is silently
//! dropped. See AMUX-2977.

use super::AppState;
use crate::db::WriteOutcome;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apply", post(apply))
        .route("/schema", get(schema))
}

// ---- the accepted shape ----------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct EnvSpec {
    #[serde(default)]
    groups: Vec<GroupSpec>,
    #[serde(default)]
    workers: Vec<WorkerSpec>,
    // Phase-2 stanzas: parsed (so a spec that includes them is not rejected)
    // and REPORTED as not-yet-applied rather than silently ignored.
    #[serde(default)]
    schedules: Vec<Value>,
    #[serde(default)]
    columns: Vec<Value>,
    #[serde(default)]
    files: Vec<Value>,
    #[serde(default)]
    global: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct GroupSpec {
    name: String,
    #[serde(default)]
    department: String,
    #[serde(default)]
    goal: String,
}

#[derive(Debug, Deserialize)]
struct WorkerSpec {
    name: String,
    #[serde(default)]
    dir: String,
    /// Group names -> CC_TAGS (comma-joined). amux's group membership.
    #[serde(default)]
    groups: Vec<String>,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    provider: String,
}

// ---- GET /api/env/schema ---------------------------------------------------

async fn schema() -> Response {
    Json(json!({
        "applied_now": ["groups", "workers"],
        "phase_2": ["schedules", "columns", "gates", "files", "global"],
        "example": {
            "groups": [{"name": "engineering", "department": "Engineering", "goal": "Ship the platform"}],
            "workers": [{
                "name": "backend-dev", "dir": "/path/to/repo", "groups": ["engineering"],
                "desc": "Backend API work", "model": "sonnet", "provider": "claude"
            }]
        },
        "idempotent": "a group is its name, a worker is its env file — re-applying converges, never duplicates",
        "content_type": "text/yaml | application/x-yaml | application/json",
    }))
    .into_response()
}

// ---- POST /api/env/apply ---------------------------------------------------

#[derive(Deserialize)]
struct ApplyQ {
    #[serde(default)]
    dry_run: u8,
}

async fn apply(
    State(state): State<AppState>,
    Query(q): Query<ApplyQ>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let dry = q.dry_run != 0;
    let ct = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let raw = String::from_utf8_lossy(&body);
    // JSON is a subset of YAML for serde_yaml, but honor an explicit JSON type.
    let spec: EnvSpec = if ct.contains("json") {
        match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => return bad(format!("invalid JSON: {e}")),
        }
    } else {
        match serde_yaml::from_str(&raw) {
            Ok(s) => s,
            Err(e) => return bad(format!("invalid YAML: {e}")),
        }
    };

    let home = crate::config::amux_home();
    let sessions_dir = home.join("sessions");

    let mut report = Vec::<Value>::new();

    // ---- workers: an env file per worker (idempotent write) ----------------
    // Validate first so a dry-run reports the same refusals an apply would hit.
    let mut worker_writes: Vec<(std::path::PathBuf, String, String, &str)> = vec![];
    for w in &spec.workers {
        let name = sanitize(&w.name);
        if name.is_empty() {
            report.push(json!({"kind": "worker", "name": w.name, "action": "error", "detail": "invalid name"}));
            continue;
        }
        if !w.dir.is_empty() && !std::path::Path::new(&w.dir).is_dir() {
            report.push(json!({"kind": "worker", "name": name, "action": "error", "detail": format!("dir does not exist: {}", w.dir)}));
            continue;
        }
        let path = sessions_dir.join(format!("{name}.env"));
        let existed = path.exists();
        let action = if existed { "update" } else { "create" };
        let content = render_worker_env(w);
        // "unchanged" if the file already holds this exact config (minus the
        // volatile `# updated:` header line) — so a re-apply reports honestly.
        let action = if existed && same_env_body(&path, &content) { "unchanged" } else { action };
        report.push(json!({"kind": "worker", "name": name, "action": action, "groups": w.groups}));
        if !dry && action != "unchanged" {
            worker_writes.push((path, content, name.clone(), action));
        }
    }

    // ---- groups: group_config upsert ---------------------------------------
    let groups_for_write: Vec<(String, String, String)> = spec
        .groups
        .iter()
        .map(|g| (g.name.trim().to_string(), g.department.clone(), g.goal.clone()))
        .filter(|(n, _, _)| !n.is_empty())
        .collect();
    // For the report, read current group_config so we can say create/update/unchanged.
    let existing_groups: std::collections::HashMap<String, (String, String)> = state
        .store
        .read()
        .ok()
        .map(|conn| {
            let mut m = std::collections::HashMap::new();
            if let Ok(mut st) = conn.prepare("SELECT name, department, goal FROM group_config") {
                if let Ok(rows) = st.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                }) {
                    for row in rows.flatten() {
                        m.insert(row.0, row.1);
                    }
                }
            }
            m
        })
        .unwrap_or_default();
    for (name, dept, goal) in &groups_for_write {
        let action = match existing_groups.get(name) {
            Some((d, g)) if d == dept && g == goal => "unchanged",
            Some(_) => "update",
            None => "create",
        };
        report.push(json!({"kind": "group", "name": name, "action": action}));
    }

    // Phase-2 stanzas: announce, don't drop.
    for (stanza, items) in [
        ("schedules", spec.schedules.len()),
        ("columns", spec.columns.len()),
        ("files", spec.files.len()),
    ] {
        if items > 0 {
            report.push(json!({"kind": stanza, "action": "not-yet-applied", "count": items,
                "detail": "phase 2 (AMUX-2977) — parsed and reported, not written"}));
        }
    }
    if spec.global.is_some() {
        report.push(json!({"kind": "global", "action": "not-yet-applied",
            "detail": "phase 2 — server.env writes need a restart, deliberately not automatic"}));
    }

    if dry {
        return Json(json!({"dry_run": true, "report": report})).into_response();
    }

    // ---- APPLY (writes) ----------------------------------------------------
    let mut errors = vec![];
    for (path, content, name, _action) in worker_writes {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = write_env_atomic(&path, &content) {
            errors.push(json!({"kind": "worker", "name": name, "error": e.to_string()}));
        }
    }
    // A worker create/delete changes the fleet registry — invalidate the cache
    // (the AMUX-2960 discipline) so the new workers show up immediately.
    super::sessions_legacy::invalidate_sessions_cache();

    if !groups_for_write.is_empty() {
        let gw = groups_for_write.clone();
        let _ = state
            .store
            .write_async(move |conn| {
                let now = chrono::Utc::now().timestamp();
                for (name, dept, goal) in &gw {
                    conn.execute(
                        "INSERT INTO group_config (name, department, goal, updated) VALUES (?1,?2,?3,?4) \
                         ON CONFLICT(name) DO UPDATE SET department=?2, goal=?3, updated=?4",
                        rusqlite::params![name, dept, goal, now],
                    )?;
                }
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .await;
    }

    Json(json!({
        "applied": true,
        "report": report,
        "errors": errors,
    }))
    .into_response()
}

// ---- helpers ---------------------------------------------------------------

fn bad(msg: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}

fn sanitize(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect()
}

/// The env-file body a worker spec produces (K="V" lines, no volatile header).
fn render_worker_env(w: &WorkerSpec) -> String {
    let mut pairs: Vec<(&str, String)> = vec![];
    if !w.dir.is_empty() {
        pairs.push(("CC_DIR", w.dir.clone()));
    }
    if !w.groups.is_empty() {
        pairs.push(("CC_TAGS", w.groups.join(",")));
    }
    if !w.desc.is_empty() {
        pairs.push(("CC_DESC", w.desc.clone()));
    }
    let provider = if w.provider.is_empty() { "claude".into() } else { w.provider.clone() };
    if provider != "claude" {
        pairs.push(("CC_PROVIDER", provider));
    }
    if !w.model.is_empty() {
        pairs.push(("CC_FLAGS", format!("--model {}", w.model)));
    }
    pairs.iter().map(|(k, v)| format!("{k}=\"{v}\"")).collect::<Vec<_>>().join("\n")
}

/// True if the existing env file's body (ignoring the `# updated:` header)
/// already equals `content` — so a re-apply reports "unchanged", not "update".
fn same_env_body(path: &std::path::Path, content: &str) -> bool {
    let Ok(existing) = std::fs::read_to_string(path) else { return false };
    let strip = |s: &str| -> String {
        s.lines()
            .filter(|l| !l.trim_start().starts_with("# updated:"))
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };
    strip(&existing) == strip(content)
}

/// Write the env file the same way create_session_legacy does: `# updated:`
/// header, 0600, atomic rename.
fn write_env_atomic(path: &std::path::Path, body: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let out = format!(
        "# updated: {}\n{}\n",
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f"),
        body
    );
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
