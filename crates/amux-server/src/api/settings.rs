//! Settings API: the four `/api/settings/*` endpoints the SPA's Settings tab
//! calls, ported from amux-server.py (`/api/settings/default-model`,
//! `/api/settings/commit-guard`, `/api/settings/task-guard`,
//! `/api/settings/env`).
//!
//! All four read/write env FILES under the amux home — `server.env` and
//! `defaults.env` — not the database, so none of them touch AppState. The
//! home is resolved per-request from `$AMUX_HOME` (legacy `$CC_HOME`), the
//! same rule `config.rs` uses, which is also how tests point writes at a
//! temp home instead of the live `~/.amux`.
//!
//! Python parity decisions, recorded so they are not "fixed" later:
//! - Python loads server.env into `os.environ` at boot (non-empty values
//!   OVERRIDE process env) and every PATCH mutates `os.environ` for "live
//!   effect". Rust has no safe process-global env mutation story, so reads
//!   go file-first instead: a non-empty server.env value wins, then process
//!   env, then the default. Observable behavior matches Python: a PATCH is
//!   immediately visible to the next GET without a restart.
//! - The Python ANTHROPIC_API_KEY PATCH also re-inits claude config and
//!   pushes the key into every running tmux session. That is the Python
//!   server's runtime to manage — while it owns the tmux fleet (Phase 11
//!   cutover pending), duplicating the push here would race it. Not ported.
//! - `defaults.env` writes are atomic with mode 0600 (Python's
//!   `_atomic_write_secure`); `server.env` writes are plain rewrites
//!   (Python's are too).
//! - Flag surgery (`--model X` / `--model=X`) uses a POSIX shlex
//!   split/quote port so quoted multi-word values survive, and malformed
//!   flags fail loudly with Python's exact 400 message instead of wiping
//!   the user's other flags.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use super::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/default-model", get(get_default_model_h).patch(patch_default_model))
        .route("/commit-guard", get(get_commit_guard).patch(patch_commit_guard))
        .route("/task-guard", get(get_task_guard).patch(patch_task_guard))
        .route("/env", get(get_env).patch(patch_env))
}

fn err(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// The amux home for settings/journal/history file access: `$AMUX_HOME`,
/// legacy `$CC_HOME`, else `~/.amux` — the same resolution as Python's
/// `_amux_home` and config.rs's `from_process_env`. Resolved per-request so
/// tests can inject a temp home.
pub(crate) fn amux_home() -> PathBuf {
    for var in ["AMUX_HOME", "CC_HOME"] {
        if let Ok(h) = std::env::var(var) {
            if !h.is_empty() {
                return PathBuf::from(h);
            }
        }
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
        .join(".amux")
}

/// Effective value of a server-config key: non-empty `server.env` entry
/// first (Python's boot loader only overrides `os.environ` with non-empty
/// values), then process env, then None. File-first is what makes a PATCH
/// visible to the next GET without mutating process env.
/// `pub(crate)`: the alert-config endpoints (api/alerts.rs) read the same
/// keys the same way — one resolver, not two spellings of it.
pub(crate) fn effective_env(home: &Path, key: &str) -> Option<String> {
    let file_env = crate::config::parse_env_file(&home.join("server.env"));
    if let Some(v) = file_env.get(key) {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Python's server.env line-replace: rewrite the first `KEY=`/`KEY =` line,
/// else append. Non-atomic plain write, matching Python (`_env_set`).
/// `pub(crate)`: shared with the alert-config PATCH (api/alerts.rs), which
/// is Python's `_env_set` on the same file.
pub(crate) fn set_server_env_key(home: &Path, key: &str, val: &str) -> std::io::Result<()> {
    let file = home.join("server.env");
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default();
    let mut found = false;
    for line in lines.iter_mut() {
        if line.starts_with(&format!("{key}=")) || line.starts_with(&format!("{key} =")) {
            *line = format!("{key}={val}");
            found = true;
            break;
        }
    }
    if !found {
        lines.push(format!("{key}={val}"));
    }
    std::fs::create_dir_all(home)?;
    std::fs::write(&file, lines.join("\n") + "\n")
}

/// Python's `_atomic_write_secure`: temp file in the same dir, chmod 0600,
/// rename over the target — no TOCTOU window, no partially-written file.
fn atomic_write_secure(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("env"),
        std::process::id()
    ));
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// JSON truthiness with Python `bool()` semantics — the guard PATCHes run
/// the body value through `bool(...)`, so `0`, `""`, `[]`, `{}` disable.
pub(crate) fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// ---- shlex port (Python shlex.split / shlex.quote, POSIX mode) ------------

/// POSIX-mode `shlex.split`. Errors with Python's message on an unclosed
/// quote so the 400 the user sees names the same problem.
pub(crate) fn shlex_split(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            '\'' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("No closing quotation".into()),
                    }
                }
            }
            '"' => {
                in_token = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        // Inside double quotes, backslash escapes only \" and
                        // \\ (Python shlex posix rules); otherwise it is kept.
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            Some(other) => {
                                cur.push('\\');
                                cur.push(other);
                            }
                            None => return Err("No closing quotation".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("No closing quotation".into()),
                    }
                }
            }
            '\\' => {
                in_token = true;
                match chars.next() {
                    Some(ch) => cur.push(ch),
                    None => return Err("No escaped character".into()),
                }
            }
            ch => {
                in_token = true;
                cur.push(ch);
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

/// Python `shlex.quote`: safe charset passes through, everything else gets
/// single-quoted with the `'"'"'` dance.
pub(crate) fn shlex_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let safe = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if s.chars().all(safe) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r#"'"'"'"#))
}

/// Python `_strip_model_from_flags`: remove `--model X` / `--model=X`,
/// re-quote the rest. Err on malformed input — the caller MUST surface it
/// rather than silently wiping the user's flags.
pub(crate) fn strip_model_from_flags(flags: &str) -> Result<String, String> {
    if flags.is_empty() {
        return Ok(String::new());
    }
    let tokens = shlex_split(flags)?;
    let mut filtered: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == "--model" && i + 1 < tokens.len() {
            i += 2;
            continue;
        }
        if t.starts_with("--model=") {
            i += 1;
            continue;
        }
        filtered.push(t.clone());
        i += 1;
    }
    Ok(filtered.iter().map(|t| shlex_quote(t)).collect::<Vec<_>>().join(" "))
}

/// Python `_extract_model_from_flags`: read-only, so malformed input
/// silently yields "" (display fallback, not surgery).
pub(crate) fn extract_model_from_flags(flags: &str) -> String {
    if flags.is_empty() {
        return String::new();
    }
    let Ok(tokens) = shlex_split(flags) else {
        return String::new();
    };
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == "--model" && i + 1 < tokens.len() {
            return tokens[i + 1].clone();
        }
        if let Some(v) = t.strip_prefix("--model=") {
            return v.to_string();
        }
        i += 1;
    }
    String::new()
}

const MODEL_ID_MAX_LEN: usize = 255;

/// Python `_validate_model_name`: string, <=255 chars, `[A-Za-z0-9._:\[\]@/+-]+`
/// with no leading hyphen (the regex's `(?!-)` lookahead, expressed directly
/// since the regex crate has no lookahead). Empty is allowed — it means
/// "clear the override".
pub(crate) fn validate_model_name(v: &Value) -> Result<String, String> {
    let Value::String(s) = v else {
        return Err("model must be a string".into());
    };
    let normalized = s.trim().to_string();
    if normalized.len() > MODEL_ID_MAX_LEN {
        return Err(format!("model name too long (max {MODEL_ID_MAX_LEN} chars)"));
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._:[]@/+-".contains(c);
    if !normalized.is_empty() && (normalized.starts_with('-') || !normalized.chars().all(allowed)) {
        return Err(
            "invalid model name (allowed: alphanumeric and ._:[]@/+-, no leading hyphen)".into(),
        );
    }
    Ok(normalized)
}

/// Python `_get_default_model`: `--model` out of defaults.env's
/// CC_DEFAULT_FLAGS, falling back to "sonnet".
pub(crate) fn get_default_model(home: &Path) -> String {
    let defaults = home.join("defaults.env");
    if defaults.exists() {
        let cfg = crate::config::parse_env_file(&defaults);
        let model = extract_model_from_flags(cfg.get("CC_DEFAULT_FLAGS").map(String::as_str).unwrap_or(""));
        if !model.is_empty() {
            return model;
        }
    }
    "sonnet".into()
}

/// The PATCH body's model applied to defaults.env — Python's handler, line
/// for line: single read (no TOCTOU), strip the old `--model` while
/// PRESERVING every other flag, quote-wrap, atomic 0600 write.
pub(crate) fn patch_default_model_file(home: &Path, model: &str) -> Result<(), (StatusCode, Value)> {
    let defaults = home.join("defaults.env");
    let mut lines: Vec<String> = if defaults.exists() {
        std::fs::read_to_string(&defaults)
            .map(|s| s.lines().map(String::from).collect())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))?
    } else {
        Vec::new()
    };
    let mut existing_flags = String::new();
    for line in &lines {
        if let Some(value) = line.strip_prefix("CC_DEFAULT_FLAGS=") {
            let mut value = value;
            // Strip outer matching quotes (mirrors parse_env_file).
            let bytes = value.as_bytes();
            if bytes.len() >= 2
                && bytes[0] == bytes[bytes.len() - 1]
                && (bytes[0] == b'"' || bytes[0] == b'\'')
            {
                value = &value[1..value.len() - 1];
            }
            existing_flags = value.to_string();
            break;
        }
    }
    let flags_no_model = strip_model_from_flags(&existing_flags).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": format!(
                "existing CC_DEFAULT_FLAGS in defaults.env is malformed ({e}); fix the file manually before updating the model via API"
            ) }),
        )
    })?;
    let new_flag_value = if !model.is_empty() {
        if !flags_no_model.is_empty() {
            format!("--model {model} {flags_no_model}").trim().to_string()
        } else {
            format!("--model {model}")
        }
    } else {
        flags_no_model
    };
    let new_line = format!("CC_DEFAULT_FLAGS=\"{new_flag_value}\"");
    let mut found = false;
    for line in lines.iter_mut() {
        if line.starts_with("CC_DEFAULT_FLAGS=") {
            *line = new_line.clone();
            found = true;
            break;
        }
    }
    if !found {
        lines.push(new_line);
    }
    let content = lines.join("\n") + "\n";
    atomic_write_secure(&defaults, &content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })))
}

// ---- /api/settings/default-model ------------------------------------------

async fn get_default_model_h() -> Response {
    Json(json!({ "model": get_default_model(&amux_home()) })).into_response()
}

async fn patch_default_model(Json(body): Json<Value>) -> Response {
    if !body.is_object() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "payload must be a JSON object" }));
    }
    // Python: body.get("model", "") — absent means "clear the override".
    let model_v = body.get("model").cloned().unwrap_or_else(|| Value::String(String::new()));
    let model = match validate_model_name(&model_v) {
        Ok(m) => m,
        Err(e) => return err(StatusCode::BAD_REQUEST, json!({ "error": e })),
    };
    match patch_default_model_file(&amux_home(), &model) {
        Ok(()) => Json(json!({ "ok": true, "model": model })).into_response(),
        Err((status, body)) => err(status, body),
    }
}

// ---- /api/settings/commit-guard and /api/settings/task-guard ---------------

/// Python `_commit_guard_enabled`: default ON, disabled only by an explicit
/// falsy spelling.
pub(crate) fn commit_guard_enabled(home: &Path) -> bool {
    let val = effective_env(home, "AMUX_COMMIT_GUARD").unwrap_or_else(|| "1".into());
    !matches!(val.trim().to_lowercase().as_str(), "0" | "false" | "off" | "no")
}

/// Python `_task_guard_enabled`: default OFF, opt-in spelling required.
pub(crate) fn task_guard_enabled(home: &Path) -> bool {
    let val = effective_env(home, "AMUX_TASK_GUARD").unwrap_or_else(|| "0".into());
    matches!(val.trim().to_lowercase().as_str(), "1" | "true" | "on" | "yes")
}

async fn get_commit_guard() -> Response {
    Json(json!({ "enabled": commit_guard_enabled(&amux_home()) })).into_response()
}

async fn patch_commit_guard(Json(body): Json<Value>) -> Response {
    // Python: bool(body.get("enabled", True)).
    let enabled = body.get("enabled").map(truthy).unwrap_or(true);
    let val = if enabled { "1" } else { "0" };
    match set_server_env_key(&amux_home(), "AMUX_COMMIT_GUARD", val) {
        Ok(()) => Json(json!({ "ok": true, "enabled": enabled })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
    }
}

async fn get_task_guard() -> Response {
    Json(json!({ "enabled": task_guard_enabled(&amux_home()) })).into_response()
}

async fn patch_task_guard(Json(body): Json<Value>) -> Response {
    // Python: bool(body.get("enabled", False)).
    let enabled = body.get("enabled").map(truthy).unwrap_or(false);
    let val = if enabled { "1" } else { "0" };
    match set_server_env_key(&amux_home(), "AMUX_TASK_GUARD", val) {
        Ok(()) => Json(json!({ "ok": true, "enabled": enabled })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
    }
}

// ---- /api/settings/env ------------------------------------------------------

/// The only keys the settings UI may read (masked) or write. Fixed array,
/// not a set: response key order is stable.
const ALLOWED_ENV_KEYS: [&str; 4] =
    ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"];

/// Python's mask: >8 chars shows stars + last 4; short-but-set shows "set";
/// unset shows "". Never the value itself.
pub(crate) fn mask_secret(v: &str) -> String {
    let n = v.chars().count();
    if n > 8 {
        let last4: String = v.chars().skip(n - 4).collect();
        format!("{}{last4}", "*".repeat(n - 4))
    } else if n > 0 {
        "set".into()
    } else {
        String::new()
    }
}

async fn get_env() -> Response {
    let home = amux_home();
    let mut out = Map::new();
    for k in ALLOWED_ENV_KEYS {
        let v = effective_env(&home, k).unwrap_or_default();
        out.insert(k.to_string(), Value::String(mask_secret(&v)));
    }
    Json(Value::Object(out)).into_response()
}

async fn patch_env(Json(body): Json<Value>) -> Response {
    let updates: Vec<(String, String)> = body
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, v)| ALLOWED_ENV_KEYS.contains(&k.as_str()) && v.is_string())
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    if updates.is_empty() {
        return err(StatusCode::BAD_REQUEST, json!({ "error": "no valid keys" }));
    }
    let home = amux_home();
    for (key, val) in &updates {
        if let Err(e) = set_server_env_key(&home, key, val) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() }));
        }
    }
    // Python also mutates os.environ and pushes ANTHROPIC_API_KEY into
    // running tmux sessions; see the module doc for why neither is ported.
    Json(json!({ "ok": true })).into_response()
}

// ---------------------------------------------------------------------------
// Shared test plumbing: AMUX_HOME is process-global, so every test that sets
// it (here, journal media, history group) must hold this lock, and the RAII
// guard restores the previous value even on panic. NEVER point it at the
// real ~/.amux.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_env {
    use std::sync::{Mutex, MutexGuard};

    pub static LOCK: Mutex<()> = Mutex::new(());

    /// Keys that leak from the REAL machine into a temp-home test.
    ///
    /// `ServerConfig::load` exports server.env into the PROCESS env
    /// (config.rs — deliberate, it is the python setdefault parity that made
    /// server.env flags actually work). But `effective_env` falls back to the
    /// process env when a key is absent from the home's file, so once ANY test
    /// loads the real ~/.amux, that machine's values are visible to every later
    /// test — whatever home they set.
    ///
    /// That made `owner_alert_respects_channel_config` fail roughly 1 run in 3
    /// under `cargo test --workspace`: it asserts "no channels configured", and
    /// found the developer's real AMUX_OWNER_PHONE, so the alert went out over
    /// sms. Order-dependent, hence intermittent, hence read as "flaky test"
    /// rather than "the test can see the machine".
    ///
    /// A temp home must mean a clean slate. Cleared here rather than in each
    /// test because the guard already holds LOCK, so this is the one place the
    /// mutation is race-free. Restored on drop.
    const LEAKY_KEYS: &[&str] = &["AMUX_OWNER_PHONE"];

    pub struct HomeGuard {
        prev: Option<String>,
        prev_leaky: Vec<(&'static str, Option<String>)>,
        _g: MutexGuard<'static, ()>,
    }

    pub fn set_home(path: &std::path::Path) -> HomeGuard {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_leaky: Vec<(&'static str, Option<String>)> = LEAKY_KEYS
            .iter()
            .map(|k| {
                let was = std::env::var(*k).ok();
                std::env::remove_var(k);
                (*k, was)
            })
            .collect();
        let prev = std::env::var("AMUX_HOME").ok();
        std::env::set_var("AMUX_HOME", path);
        HomeGuard {
            prev,
            prev_leaky,
            _g: g,
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var("AMUX_HOME", v),
                None => std::env::remove_var("AMUX_HOME"),
            }
            for (k, was) in &self.prev_leaky {
                match was {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app() -> (axum::Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::db::Store::open(&dir.path().join("settings-test.db")).unwrap();
        let state = AppState {
            store: std::sync::Arc::new(store),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        let router = Router::new().nest("/api/settings", routes()).with_state(state);
        (router, dir)
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let b = Request::builder().method(method).uri(path);
        let req = match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, v)
    }

    #[test]
    fn shlex_split_matches_python_shapes() {
        assert_eq!(shlex_split("--model opus --max-tokens 8000").unwrap(),
                   vec!["--model", "opus", "--max-tokens", "8000"]);
        assert_eq!(shlex_split(r#"--append-system-prompt "be very terse""#).unwrap(),
                   vec!["--append-system-prompt", "be very terse"]);
        assert_eq!(shlex_split("--x 'a b'").unwrap(), vec!["--x", "a b"]);
        assert_eq!(shlex_split("").unwrap(), Vec::<String>::new());
        assert_eq!(shlex_split(r#"a\ b"#).unwrap(), vec!["a b"]);
        // Unbalanced quote errors with Python's message.
        assert_eq!(shlex_split(r#"--x "unclosed"#).unwrap_err(), "No closing quotation");
    }

    #[test]
    fn shlex_quote_matches_python() {
        assert_eq!(shlex_quote("opus"), "opus");
        assert_eq!(shlex_quote("--max-tokens"), "--max-tokens");
        assert_eq!(shlex_quote("a b"), "'a b'");
        assert_eq!(shlex_quote(""), "''");
        assert_eq!(shlex_quote("it's"), r#"'it'"'"'s'"#);
    }

    #[test]
    fn model_flag_surgery_preserves_other_flags() {
        assert_eq!(strip_model_from_flags("--model opus --max-tokens 8000").unwrap(),
                   "--max-tokens 8000");
        assert_eq!(strip_model_from_flags("--model=opus --effort high").unwrap(),
                   "--effort high");
        assert_eq!(strip_model_from_flags("").unwrap(), "");
        // Quoted multi-word values survive re-quoting.
        assert_eq!(
            strip_model_from_flags(r#"--model opus --append-system-prompt "be terse""#).unwrap(),
            "--append-system-prompt 'be terse'"
        );
        assert!(strip_model_from_flags(r#"--model "unclosed"#).is_err());

        assert_eq!(extract_model_from_flags("--model opus --x y"), "opus");
        assert_eq!(extract_model_from_flags("--model=claude-fable-5"), "claude-fable-5");
        assert_eq!(extract_model_from_flags("--x y"), "");
        assert_eq!(extract_model_from_flags(r#"--model "unclosed"#), "");
    }

    #[test]
    fn model_name_validation_matches_python() {
        assert_eq!(validate_model_name(&json!("  opus  ")).unwrap(), "opus");
        assert_eq!(validate_model_name(&json!("")).unwrap(), "");
        assert_eq!(validate_model_name(&json!("us.anthropic.claude-3[1m]@x/+y")).unwrap(),
                   "us.anthropic.claude-3[1m]@x/+y");
        assert!(validate_model_name(&json!(3)).is_err());
        assert!(validate_model_name(&json!("-leading-hyphen")).is_err());
        assert!(validate_model_name(&json!("has space")).is_err());
        assert!(validate_model_name(&json!("x".repeat(256))).is_err());
    }

    #[test]
    fn mask_matches_python() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short"), "set");
        assert_eq!(mask_secret("12345678"), "set");
        assert_eq!(mask_secret("sk-ant-api03-abcd"), "*************abcd");
    }

    #[test]
    fn default_model_file_helpers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // Empty home: sonnet fallback.
        assert_eq!(get_default_model(home), "sonnet");
        // Patch a model in.
        patch_default_model_file(home, "opus").unwrap();
        assert_eq!(get_default_model(home), "opus");
        assert_eq!(
            std::fs::read_to_string(home.join("defaults.env")).unwrap(),
            "CC_DEFAULT_FLAGS=\"--model opus\"\n"
        );
        // Other flags and other lines survive a model change.
        std::fs::write(
            home.join("defaults.env"),
            "OTHER=1\nCC_DEFAULT_FLAGS=\"--model sonnet --max-tokens 8000\"\n",
        )
        .unwrap();
        patch_default_model_file(home, "opus").unwrap();
        let content = std::fs::read_to_string(home.join("defaults.env")).unwrap();
        assert_eq!(content, "OTHER=1\nCC_DEFAULT_FLAGS=\"--model opus --max-tokens 8000\"\n");
        // Clearing the model keeps the rest.
        patch_default_model_file(home, "").unwrap();
        let content = std::fs::read_to_string(home.join("defaults.env")).unwrap();
        assert_eq!(content, "OTHER=1\nCC_DEFAULT_FLAGS=\"--max-tokens 8000\"\n");
        // 0600 like Python's _atomic_write_secure.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join("defaults.env")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Malformed existing flags: loud 400, file untouched.
        std::fs::write(home.join("defaults.env"), "CC_DEFAULT_FLAGS=\"--model 'unclosed\"\n").unwrap();
        let e = patch_default_model_file(home, "opus").unwrap_err();
        assert_eq!(e.0, StatusCode::BAD_REQUEST);
        assert!(e.1["error"].as_str().unwrap().contains("malformed"), "{:?}", e.1);
        assert!(e.1["error"].as_str().unwrap().contains("fix the file manually"));
    }

    #[test]
    fn guard_helpers_defaults_and_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // Defaults: commit ON, task OFF.
        assert!(commit_guard_enabled(home));
        assert!(!task_guard_enabled(home));
        // Explicit falsy spellings disable commit-guard.
        set_server_env_key(home, "AMUX_COMMIT_GUARD", "off").unwrap();
        assert!(!commit_guard_enabled(home));
        // Junk is NOT a falsy spelling — commit-guard stays on.
        set_server_env_key(home, "AMUX_COMMIT_GUARD", "banana").unwrap();
        assert!(commit_guard_enabled(home));
        // Task-guard needs an explicit truthy spelling.
        set_server_env_key(home, "AMUX_TASK_GUARD", "banana").unwrap();
        assert!(!task_guard_enabled(home));
        set_server_env_key(home, "AMUX_TASK_GUARD", "yes").unwrap();
        assert!(task_guard_enabled(home));
        // Line-replace, not append-forever.
        let content = std::fs::read_to_string(home.join("server.env")).unwrap();
        assert_eq!(content.matches("AMUX_TASK_GUARD").count(), 1, "{content}");
    }

    #[tokio::test]
    async fn settings_endpoints_end_to_end_in_temp_home() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = test_env::set_home(dir.path());
        let (app, _dbdir) = app();

        // default-model GET (fallback) / PATCH / GET.
        let (st, v) = send(&app, "GET", "/api/settings/default-model", None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["model"], json!("sonnet"));
        let (st, v) =
            send(&app, "PATCH", "/api/settings/default-model", Some(json!({ "model": "opus" }))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v, json!({ "ok": true, "model": "opus" }));
        let (_, v) = send(&app, "GET", "/api/settings/default-model", None).await;
        assert_eq!(v["model"], json!("opus"));
        // Bad payloads.
        let (st, v) = send(&app, "PATCH", "/api/settings/default-model", Some(json!(["x"]))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], json!("payload must be a JSON object"));
        let (st, _) =
            send(&app, "PATCH", "/api/settings/default-model", Some(json!({ "model": "-x" }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);

        // Guards: GET defaults, PATCH, GET reflects the file immediately.
        let (_, v) = send(&app, "GET", "/api/settings/commit-guard", None).await;
        assert_eq!(v, json!({ "enabled": true }));
        let (_, v) =
            send(&app, "PATCH", "/api/settings/commit-guard", Some(json!({ "enabled": false }))).await;
        assert_eq!(v, json!({ "ok": true, "enabled": false }));
        let (_, v) = send(&app, "GET", "/api/settings/commit-guard", None).await;
        assert_eq!(v, json!({ "enabled": false }));
        // Python default when the key is absent from the body: commit=true, task=false.
        let (_, v) = send(&app, "PATCH", "/api/settings/commit-guard", Some(json!({}))).await;
        assert_eq!(v["enabled"], json!(true));
        let (_, v) = send(&app, "PATCH", "/api/settings/task-guard", Some(json!({}))).await;
        assert_eq!(v["enabled"], json!(false));
        let (_, v) =
            send(&app, "PATCH", "/api/settings/task-guard", Some(json!({ "enabled": true }))).await;
        assert_eq!(v, json!({ "ok": true, "enabled": true }));
        let (_, v) = send(&app, "GET", "/api/settings/task-guard", None).await;
        assert_eq!(v, json!({ "enabled": true }));

        // env: masked GET, allow-listed PATCH.
        //
        // This endpoint reports the PROCESS environment, not just the temp
        // home's server.env — deliberately, since that is what a worker would
        // actually receive. So a bare `assert_eq!(.., "")` is not a statement
        // about the code, it is a statement about whoever ran the test: it
        // passed in CI (no OPENAI_API_KEY) and failed on a dev machine that had
        // one exported, which reads as "main is red" when nothing is broken.
        //
        // Assert the real contract instead — unset reads empty, set reads
        // MASKED and never leaks the value — and branch on the ambient env
        // rather than mutating it, because `set_var` is process-global and this
        // suite runs threaded (the alerts tests already race on exactly that).
        let (_, v) = send(&app, "GET", "/api/settings/env", None).await;
        match std::env::var("OPENAI_API_KEY") {
            Err(_) => assert_eq!(v["OPENAI_API_KEY"], json!("")),
            Ok(real) => {
                let shown = v["OPENAI_API_KEY"].as_str().unwrap_or_default();
                assert!(
                    shown.starts_with('*'),
                    "a configured key must come back masked, got {shown:?}"
                );
                assert!(
                    !shown.contains(&real),
                    "the masked form must never contain the real value"
                );
            }
        }
        let (st, v) = send(
            &app,
            "PATCH",
            "/api/settings/env",
            Some(json!({ "ANTHROPIC_API_KEY": "sk-ant-api03-abcd", "NOT_ALLOWED": "x" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v, json!({ "ok": true }));
        let (_, v) = send(&app, "GET", "/api/settings/env", None).await;
        assert_eq!(v["ANTHROPIC_API_KEY"], json!("*************abcd"));
        // The disallowed key never reached the file.
        let content = std::fs::read_to_string(dir.path().join("server.env")).unwrap();
        assert!(content.contains("ANTHROPIC_API_KEY=sk-ant-api03-abcd"));
        assert!(!content.contains("NOT_ALLOWED"));
        // Only disallowed / non-string keys: Python's 400.
        let (st, v) =
            send(&app, "PATCH", "/api/settings/env", Some(json!({ "NOT_ALLOWED": "x" }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], json!("no valid keys"));
        let (st, _) =
            send(&app, "PATCH", "/api/settings/env", Some(json!({ "OPENAI_API_KEY": 42 }))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }
}
