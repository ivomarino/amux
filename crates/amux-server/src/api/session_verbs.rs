//! /api/sessions/{name} + /api/sessions/{name}/{verb} — NATIVE session verbs
//! (AMUX-2598 cutover: "the rust version isn't using any python, just rust").
//!
//! This retires row 1 of py_proxy::PROXIED_FAMILIES. Every verb the SPA calls
//! is answered from Rust against the same fleet substrate Python manages:
//! `~/.amux/sessions/<name>.env` (registry), `<name>.meta.json` (meta),
//! `~/.amux/logs/<name>.log` (pipe-pane logs), tmux sessions named
//! `amux-<name>`, and the shared SQLite DB (steering_queue, steering_history,
//! share_tokens, session_events, cmd_history, send_dedup, issues, prefs).
//!
//! Porting map (amux-server.py, line numbers checked 2026-08-09):
//! - dispatch block            py:74873-76757
//! - peek                      py:74985-75136 (shape: history/live/output +
//!   output_lines/history_lines/output_is_viewport_only/hint — AMUX-1807 and
//!   the 2026-07-27 "swallowed message" incident are load-bearing here)
//! - transcript renderer       py:5833-5957 (_render_session_transcript)
//! - send choreography         py:25432-25715 (send_text)
//! - start choreography        py:24218-24887 (start_session)
//! - stop choreography         py:24943-25054 (stop_session)
//! - config PATCH              py:76327-76755 (rename cascade, provider/model/
//!   effort/yolo restart, dir restart, desc/tags/pin/branch/mcp/new_conversation)
//! - share                     py:65953-65999
//!
//! tmux L2: every target string is built from backend::tmux::{session_target,
//! pane_target} — the exact-match `=name` vs pane-level `=name:` split that
//! took the fleet down on 2026-08-08 lives in ONE place. All `-F` formats use
//! ':' separators, never '\t' (locale sanitization incident 2026-08-09).
//!
//! Residual gaps vs Python, named honestly (each returns a correct-typed
//! response or an explicit error — never silent):
//! - no autotask/board-labelling on send (Python's model-call feature)
//! - no _verify_submitted JSONL evidence gate after send (reports "sent" once
//!   keys landed; Python additionally greps the JSONL)
//! - no boot board-digest briefing on start (standing instructions ARE re-sent)
//! - no _install_amux_commit_hook / _auto_trust_dir / _ensure_memory side
//!   effects (Python's loops still own those during coexistence)
//! - commit-report attaches to the in-flight card but skips the cross-session
//!   sweep notice (py:76008-76230)
//! - env-explain / memory-explain answer 501 with a pointer (layered env
//!   composition is not ported yet)
//! - iTerm2-backed sessions (CC_ITERM2_SESSION_ID) answer 501 (0 in the fleet)

use super::AppState;
use crate::api::fs::{body_str, parse_body, parse_qs, qs_get};
use crate::api::sessions_legacy::strip_ansi;
use crate::backend::tmux::{pane_target, session_target};
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const OP_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// Python: MAX_LOG_BYTES = 10MB (py:892).
const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Fleet paths (Python: CC_HOME/CC_SESSIONS/CC_LOGS/CC_MEMORY/CC_TRANSCRIPTS,
// py:59-69; CLAUDE_HOME py:862). Read at call time like sessions_legacy — the
// AppState-captured-home refactor is a named deviation there.
// ---------------------------------------------------------------------------

fn home() -> PathBuf {
    std::env::var("AMUX_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
    })
}
fn sessions_dir() -> PathBuf {
    home().join("sessions")
}
fn logs_dir() -> PathBuf {
    home().join("logs")
}
fn memory_dir() -> PathBuf {
    home().join("memory")
}
fn transcripts_dir() -> PathBuf {
    home().join("transcripts")
}
fn claude_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}
fn env_path(name: &str) -> PathBuf {
    sessions_dir().join(format!("{name}.env"))
}
fn meta_path(name: &str) -> PathBuf {
    sessions_dir().join(format!("{name}.meta.json"))
}
fn log_path(name: &str) -> PathBuf {
    logs_dir().join(format!("{name}.log"))
}
fn plain_log_path(name: &str) -> PathBuf {
    // Hidden subdir so it never collides with a real `<name>.log` (py:5457).
    logs_dir().join(".plain").join(format!("{name}.log"))
}
fn mem_file(name: &str) -> PathBuf {
    memory_dir().join(format!("{name}.md"))
}

/// Python's `_VALID_SESSION_NAME_RE` (py:25529): `^[a-zA-Z0-9_.\-]+$`.
fn valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn is_session_blocked(name: &str) -> bool {
    std::fs::read_to_string(home().join("blocked-sessions.txt"))
        .map(|t| {
            t.lines()
                .map(str::trim)
                .any(|l| !l.is_empty() && !l.starts_with('#') && l == name)
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Session .env I/O. Python's parse keeps dict insertion order and _write_env
// rewrites `# updated: <iso>` + K="V" with 0600 atomic replace (py:4180-4283).
// Ordered Vec so a rewrite preserves the user's key order.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct EnvFile {
    pairs: Vec<(String, String)>,
}

impl EnvFile {
    fn load(path: &Path) -> Self {
        let mut pairs = Vec::new();
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self { pairs };
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                continue;
            }
            let v = v.trim();
            let v = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
                || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
            {
                &v[1..v.len() - 1]
            } else {
                v
            };
            match pairs.iter_mut().find(|(pk, _)| pk == k) {
                Some((_, pv)) => *pv = v.to_string(),
                None => pairs.push((k.to_string(), v.to_string())),
            }
        }
        Self { pairs }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }
    fn set(&mut self, key: &str, value: &str) {
        match self.pairs.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_string(),
            None => self.pairs.push((key.to_string(), value.to_string())),
        }
    }
    fn remove(&mut self, key: &str) {
        self.pairs.retain(|(k, _)| k != key);
    }

    /// Python `_write_env` (py:4252): `# updated:` header + K="V", atomic 0600.
    fn write(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut out = format!("# updated: {}\n", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f"));
        for (k, v) in &self.pairs {
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
}

fn parse_env(name: &str) -> EnvFile {
    EnvFile::load(&env_path(name))
}

fn provider_of(cfg: &EnvFile) -> String {
    let p = cfg.get_or("CC_PROVIDER", "claude").trim().to_lowercase();
    if SESSION_PROVIDERS.contains(&p.as_str()) && !p.is_empty() {
        p
    } else {
        "claude".into()
    }
}

fn work_dir_of(cfg: &EnvFile) -> String {
    let wd = cfg.get_or("CC_DIR", "").trim();
    if wd.is_empty() {
        return String::new();
    }
    expanduser(wd)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| expanduser(wd).to_string_lossy().into_owned())
}

fn session_work_dir(name: &str) -> String {
    work_dir_of(&parse_env(name))
}

fn expanduser(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest);
    }
    if p == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default());
    }
    PathBuf::from(p)
}

// ---------------------------------------------------------------------------
// Meta I/O (py:12229-12251).
// ---------------------------------------------------------------------------

fn load_meta(name: &str) -> Map<String, Value> {
    std::fs::read_to_string(meta_path(name))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save_meta(name: &str, meta: &Map<String, Value>) {
    let _ = std::fs::create_dir_all(sessions_dir());
    let _ = std::fs::write(meta_path(name), Value::Object(meta.clone()).to_string());
}

fn update_meta(name: &str, updates: &[(&str, Value)]) {
    let mut meta = load_meta(name);
    for (k, v) in updates {
        meta.insert((*k).to_string(), v.clone());
    }
    save_meta(name, &meta);
}

fn meta_str(meta: &Map<String, Value>, key: &str) -> String {
    meta.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// tmux ops. Targets come ONLY from backend::tmux's L2 helpers; the fleet's
// tmux name is `amux-<name>` (py:4307 tmux_name — legacy cmux-/cc- migration
// dropped: nothing in the fleet carries those prefixes anymore).
// ---------------------------------------------------------------------------

fn tmux_name(name: &str) -> String {
    format!("amux-{name}")
}
/// Session-level target (`=amux-<n>`), exact match (py:4323 tmux_target notes
/// the 2026-08-08 prefix-match kill; L2 keeps the format in tmux.rs).
fn st(name: &str) -> String {
    session_target(&tmux_name(name))
}
/// Pane-level target (`=amux-<n>:`).
fn pt(name: &str) -> String {
    pane_target(&tmux_name(name))
}

async fn run_cmd(bin: &str, args: &[&str], timeout: Duration) -> Option<std::process::Output> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => Some(out),
        _ => None,
    }
}

async fn tmux(args: &[&str]) -> Option<std::process::Output> {
    run_cmd("tmux", args, OP_TIMEOUT).await
}

async fn tmux_sessions_set() -> std::collections::BTreeSet<String> {
    let Some(out) = tmux(&["list-sessions", "-F", "#{session_name}"]).await else {
        return Default::default();
    };
    if !out.status.success() {
        return Default::default();
    }
    String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).collect()
}

/// tmux capture-pane -e; lines<=0 → visible screen only (py:4406).
async fn tmux_capture(name: &str, lines: i64) -> String {
    if session_backend(name) == "herdr" {
        return herdr_capture(name, lines.max(1)).await;
    }
    let pt = pt(name);
    let start;
    let mut args = vec!["capture-pane", "-t", pt.as_str(), "-p", "-e"];
    if lines > 0 {
        start = format!("-{lines}");
        args.push("-S");
        args.push(&start);
    }
    match run_cmd("tmux", &args, CAPTURE_TIMEOUT).await {
        Some(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// `#{alternate_on}` probe (py:4480). herdr keeps scrollback under alt.
async fn tmux_alt_screen(name: &str) -> bool {
    if session_backend(name) == "herdr" {
        return false;
    }
    let pt = pt(name);
    match tmux(&["display-message", "-t", &pt, "-p", "#{alternate_on}"]).await {
        Some(out) => String::from_utf8_lossy(&out.stdout).trim() == "1",
        None => false,
    }
}

async fn send_key(name: &str, key: &str) {
    let pt = pt(name);
    let _ = tmux(&["send-keys", "-t", &pt, key]).await;
}
async fn send_literal(name: &str, text: &str) -> bool {
    let pt = pt(name);
    matches!(tmux(&["send-keys", "-t", &pt, "-l", text]).await, Some(o) if o.status.success())
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

// ---------------------------------------------------------------------------
// Backend selection (py:4673-4692). CC_BACKEND wins, then AMUX_BACKEND env.
// ---------------------------------------------------------------------------

fn backend_of_cfg(cfg: &EnvFile) -> String {
    let b = cfg.get_or("CC_BACKEND", "").trim().to_lowercase();
    if b == "herdr" || b == "tmux" {
        return b;
    }
    let ab = std::env::var("AMUX_BACKEND").unwrap_or_default().trim().to_lowercase();
    if ab == "herdr" {
        "herdr".into()
    } else {
        "tmux".into()
    }
}
fn session_backend(name: &str) -> String {
    backend_of_cfg(&parse_env(name))
}
fn iterm2_id(cfg: &EnvFile) -> String {
    cfg.get_or("CC_ITERM2_SESSION_ID", "").trim().to_string()
}

// ---------------------------------------------------------------------------
// herdr ops via the CLI (py:4700-5150). One named session (AMUX_HERDR_SESSION,
// default "amux"); agent name from CC_HERDR_AGENT or the lowercase mapping.
// ---------------------------------------------------------------------------

fn herdr_session() -> String {
    let s = std::env::var("AMUX_HERDR_SESSION").unwrap_or_default();
    let s = s.trim();
    if s.is_empty() { "amux".into() } else { s.to_string() }
}

fn herdr_agent_name(name: &str) -> String {
    let cfg = parse_env(name);
    let existing = cfg.get_or("CC_HERDR_AGENT", "").trim().to_string();
    if !existing.is_empty() {
        return existing;
    }
    // Python persists the mapping back into the env file (py:4779); reading
    // side only here — the write happens on herdr start, which stays a gap.
    let re = regex::Regex::new(r"[^a-z0-9_-]").unwrap();
    let mut mapped = re.replace_all(&name.to_lowercase(), "-").into_owned();
    let re2 = regex::Regex::new(r"-{2,}").unwrap();
    mapped = re2.replace_all(&mapped, "-").trim_matches('-').chars().take(32).collect();
    mapped = mapped.trim_matches('-').to_string();
    if mapped.is_empty() || !mapped.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false) {
        mapped = format!("a-{mapped}").chars().take(32).collect::<String>().trim_end_matches('-').to_string();
    }
    mapped
}

async fn herdr_json(args: &[&str], timeout: Duration) -> Option<Value> {
    let hs = herdr_session();
    let mut full: Vec<&str> = vec!["--session", &hs];
    full.extend_from_slice(args);
    let out = run_cmd("herdr", &full, timeout).await?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    if !v.is_object() || v.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        return None;
    }
    Some(v)
}

async fn herdr_agent_running(name: &str) -> bool {
    let an = herdr_agent_name(name);
    matches!(
        herdr_json(&["agent", "get", &an], OP_TIMEOUT).await,
        Some(v) if v["result"]["agent"].is_object()
    )
}

async fn herdr_capture(name: &str, lines: i64) -> String {
    let an = herdr_agent_name(name);
    let n = lines.max(1).to_string();
    let hs = herdr_session();
    let args = [
        "--session", hs.as_str(), "agent", "read", an.as_str(),
        "--source", "recent-unwrapped", "--lines", n.as_str(), "--format", "text",
    ];
    match run_cmd("herdr", &args, Duration::from_secs(8)).await {
        Some(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

async fn herdr_send(name: &str, text: &str) -> (bool, String) {
    if !herdr_agent_running(name).await {
        return (false, "not running".into());
    }
    let cap = herdr_capture(name, 15).await;
    if !cap.is_empty() && at_resume_picker(&cap) {
        return (false, "session is in resume picker".into());
    }
    let an = herdr_agent_name(name);
    let _ = herdr_json(&["agent", "send-keys", &an, "ctrl+u"], OP_TIMEOUT).await;
    sleep_ms(100).await;
    match herdr_json(&["agent", "prompt", &an, text], Duration::from_secs(15)).await {
        Some(_) => (true, "sent".into()),
        None => (false, "herdr prompt failed".into()),
    }
}

// ---------------------------------------------------------------------------
// Text utilities (py:5346-5455, 5958-6010).
// ---------------------------------------------------------------------------

/// Blank-run collapse, ANSI-aware (py:5359 _collapse_blank_runs, keep=1).
fn collapse_blank_runs(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut blanks = 0;
    for ln in text.split('\n') {
        if strip_ansi(ln).trim().is_empty() {
            blanks += 1;
            if blanks <= 1 {
                out.push("");
            }
        } else {
            blanks = 0;
            out.push(ln);
        }
    }
    out.join("\n")
}

/// py:5393 _strip_scroll_pill — Claude's "Jump to bottom (click) ↓" overlay.
fn strip_scroll_pill(text: &str) -> String {
    if !text.contains("Jump to bottom") {
        return text.to_string();
    }
    let re = regex::Regex::new(r"\s*Jump to bottom \(click\)\s*[↓]?\s*").unwrap();
    re.replace_all(text, " ").into_owned()
}

fn launch_markers() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"--dangerously-skip-permissions\s+--name\b|unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT|Resume this session with:|claude --resume\b|The default interactive shell is now zsh|please visit https://support\.apple\.com/kb/HT208050",
        )
        .expect("launch markers regex")
    })
}

/// py:5405 _strip_launch_noise — cut through amux's relaunch scaffolding.
fn strip_launch_noise(text: &str) -> String {
    if text.is_empty()
        || (!text.contains("--name")
            && !text.contains("Resume this session with")
            && !text.contains("shell is now zsh"))
    {
        return text.to_string();
    }
    let bare_prompt = regex::Regex::new(r"^[A-Za-z0-9._-]{1,24}\$$").unwrap();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut last: isize = -1;
    for (i, ln) in lines.iter().enumerate() {
        if launch_markers().is_match(&strip_ansi(ln)) {
            last = i as isize;
        }
    }
    if last < 0 {
        return text.to_string();
    }
    let mut j = (last + 1) as usize;
    while j < lines.len() {
        let clean = strip_ansi(lines[j]).trim().to_string();
        if clean.is_empty() || bare_prompt.is_match(&clean) {
            j += 1;
        } else {
            break;
        }
    }
    let kept = &lines[j..];
    if !kept.iter().any(|l| !strip_ansi(l).trim().is_empty()) {
        return text.to_string();
    }
    kept.join("\n")
}

fn cursor_move_re() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\x1b\[(?:\d+;\d+H|\?25[lh]|\d+[ABCD]|H)").unwrap())
}

/// py:5346 _log_looks_torn.
fn log_looks_torn(text: &str) -> bool {
    if text.len() < 2000 {
        return false;
    }
    let c = cursor_move_re().find_iter(text).count();
    c >= 20 && (c as f64) / (text.len() as f64 / 1024.0) >= 2.0
}

/// py:5958 _trim_live_overlap — live frame minus what the transcript covers.
fn trim_live_overlap(transcript: &str, live: &str) -> String {
    if transcript.is_empty() || live.is_empty() {
        return live.to_string();
    }
    fn norm(s: &str) -> String {
        let s = strip_ansi(s);
        let re = regex::Regex::new(
            "[*#`_|\u{2502}\u{250c}\u{2510}\u{2514}\u{2518}\u{251c}\u{2524}\u{252c}\u{2534}\u{253c}\u{2500}=>\u{2022}\u{00b7}\u{00bb}\u{276f}\u{23bf}\u{23fa}\u{273b}\u{2726}\u{25cf}]+",
        )
        .unwrap();
        let s = re.replace_all(&s, " ");
        let ws = regex::Regex::new(r"\s+").unwrap();
        ws.replace_all(s.trim(), " ").to_lowercase()
    }
    let tlines: Vec<&str> = transcript.split('\n').collect();
    let tail_start = tlines.len().saturating_sub(140);
    let tail_norm: Vec<String> = tlines[tail_start..]
        .iter()
        .map(|x| norm(x))
        .filter(|n| n.chars().count() >= 12)
        .collect();
    let tail_set: std::collections::BTreeSet<&str> = tail_norm.iter().map(|s| s.as_str()).collect();
    let long_tail: Vec<&String> = tail_norm.iter().filter(|n| n.chars().count() >= 46).collect();
    let in_transcript = |n: &str| -> bool {
        if n.chars().count() < 12 {
            return false;
        }
        if tail_set.contains(n) {
            return true;
        }
        if n.chars().count() >= 24 {
            for tv in &long_tail {
                if n.contains(tv.as_str()) || tv.contains(n) {
                    return true;
                }
            }
        }
        false
    };
    let ll: Vec<&str> = live.split('\n').collect();
    let matches: Vec<usize> =
        ll.iter().enumerate().filter(|(_, x)| in_transcript(&norm(x))).map(|(i, _)| i).collect();
    if matches.len() < 3 {
        return live.to_string();
    }
    let after = matches[matches.len() - 1] + 1;
    ll[after.min(ll.len())..].join("\n").trim_start_matches('\n').to_string()
}

fn chars_truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ---------------------------------------------------------------------------
// Pane-state detectors (py:8229-8330, 18466-18580). D1 territory: these are
// the scraper FALLBACK Python still runs; ported verbatim-enough that the
// send/start choreography behaves the same.
// ---------------------------------------------------------------------------

const PROMPT_GLYPHS: [char; 3] = ['\u{276f}', '\u{203a}', '>'];

fn is_prompt_line(s: &str) -> bool {
    s.chars().next().map(|c| PROMPT_GLYPHS.contains(&c)).unwrap_or(false)
}

/// py:8229 _claude_ui_visible (claude + codex + gemini markers).
fn claude_ui_visible(clean_output: &str) -> bool {
    let lines: Vec<&str> = clean_output.lines().filter(|l| !l.trim().is_empty()).collect();
    let shell_prompt = regex::Regex::new(r"^.*[$%]\s").unwrap();
    let n = lines.len();
    for l in &lines[n.saturating_sub(3)..] {
        let ls = l.trim().to_lowercase();
        if shell_prompt.is_match(&ls) {
            continue;
        }
        if l.contains("\u{23f5}\u{23f5}") || ls.contains("bypass permissions") || ls.contains("plan mode") {
            return true;
        }
        if ls.contains("codex")
            && (ls.contains("full-auto") || ls.contains("suggest") || ls.contains("workspace")
                || ls.contains("approval") || ls.contains("-a never"))
        {
            return true;
        }
    }
    for l in &lines[n.saturating_sub(12)..] {
        let s = l.trim();
        if let Some(c) = s.chars().next() {
            if ('\u{2700}'..='\u{27bf}').contains(&c) && c != '\u{276f}' {
                return true;
            }
        }
    }
    let gpt_re = regex::Regex::new(r"gpt-\d|o[34][-m]").unwrap();
    for l in &lines[n.saturating_sub(12)..] {
        let s = l.trim();
        let sl = s.to_lowercase();
        if s.starts_with('\u{2022}') && sl.contains("working") && sl.contains("esc to interrupt") {
            return true;
        }
        if s.contains('\u{00b7}') && gpt_re.is_match(s) {
            return true;
        }
    }
    let head: Vec<&str> = lines.iter().take(15).copied().collect();
    let tail20: Vec<&str> = lines[n.saturating_sub(20)..].to_vec();
    let has_codex = head.iter().chain(tail20.iter()).any(|l| l.to_lowercase().contains("codex"));
    if has_codex {
        for l in &lines[n.saturating_sub(5)..] {
            let ls = l.trim();
            if ls == ">" || ls.starts_with("> ") || ls.starts_with('\u{203a}') {
                return true;
            }
            if ls.contains('\u{00b7}') && (ls.contains("gpt-") || ls.contains("o3") || ls.contains("o4")) {
                return true;
            }
        }
    }
    let head20: Vec<&str> = lines.iter().take(20).copied().collect();
    let tail12: Vec<&str> = lines[n.saturating_sub(12)..].to_vec();
    let has_gemini =
        head20.iter().chain(tail12.iter()).any(|l| l.to_lowercase().contains("gemini"));
    if has_gemini {
        for l in &lines[n.saturating_sub(8)..] {
            let ls = l.trim().to_lowercase();
            if ls == ">" || ls.starts_with("> ") || ls.starts_with('\u{203a}') {
                return true;
            }
            if ls.contains("gemini-") || ls.contains("yolo") || ls.contains("approval") {
                return true;
            }
        }
    }
    false
}

/// py:8288 _at_resume_picker.
fn at_resume_picker(clean_output: &str) -> bool {
    !clean_output.is_empty()
        && (clean_output.contains("Resume Session")
            || clean_output.contains("Type to Search")
            || clean_output.contains("Enter to select")
            || clean_output.contains("Esc to cancel"))
        && clean_output.contains('\u{2315}')
}

/// py:8307 _at_shell_prompt.
fn at_shell_prompt(clean_output: &str) -> bool {
    if claude_ui_visible(clean_output) {
        return false;
    }
    let lines: Vec<&str> = clean_output.lines().filter(|l| !l.trim().is_empty()).collect();
    let ends = regex::Regex::new(r"[$%]\s*$").unwrap();
    let leaks = regex::Regex::new(r"^\S+[$%]\s").unwrap();
    for l in &lines[lines.len().saturating_sub(5)..] {
        let ls = l.trim();
        if ends.is_match(ls) && !ls.contains('\u{276f}') {
            return true;
        }
        if leaks.is_match(ls) && !ls.contains('\u{276f}') {
            return true;
        }
    }
    false
}

/// py:18479 _detect_claude_status → 'active' | 'waiting' | 'idle' | ''.
fn detect_claude_status(raw_output: &str) -> String {
    if raw_output.is_empty() {
        return String::new();
    }
    let clean = strip_ansi(raw_output);
    let lines: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return String::new();
    }
    let n = lines.len();
    let reading_re = regex::Regex::new(r"^Reading \d+ file").unwrap();
    // 0. Active spinner, wide window.
    for l in lines[n.saturating_sub(30)..].iter().rev() {
        let s = l.trim();
        if is_prompt_line(s) {
            continue;
        }
        if let Some(c) = s.chars().next() {
            if ('\u{2700}'..='\u{27bf}').contains(&c) && s.contains('\u{2026}') {
                return "active".into();
            }
        }
        if s.starts_with("Running\u{2026}") || reading_re.is_match(s) {
            return "active".into();
        }
    }
    // 1. Status bar in the bottom 3 lines.
    let mut status_bar = String::new();
    for l in lines[n.saturating_sub(3)..].iter().rev() {
        let ls = l.trim();
        let lsl = ls.to_lowercase();
        if ls.contains("\u{23f5}\u{23f5}") || lsl.contains("bypass permissions") || lsl.contains("plan mode") {
            status_bar = lsl;
            break;
        }
    }
    if status_bar.is_empty() {
        if clean.contains("Resume from summary") && clean.contains("Resume full session") {
            return "waiting".into();
        }
        for l in &lines[n.saturating_sub(5)..] {
            if l.to_lowercase().contains("esc to interrupt") {
                return "active".into();
            }
        }
    }
    // 2. Bottom-up scan of the last 12 lines.
    let completed_re = regex::Regex::new(r" for \d+\s*[hms]\b").unwrap();
    for l in lines[n.saturating_sub(12)..].iter().rev() {
        let s = l.trim();
        let sl = s.to_lowercase();
        if let Some(c) = s.chars().next() {
            if ('\u{2700}'..='\u{27bf}').contains(&c) && s.contains('\u{2026}') && !is_prompt_line(s) {
                return "active".into();
            }
            if ('\u{2700}'..='\u{27bf}').contains(&c)
                && !s.contains('\u{2026}')
                && completed_re.is_match(s)
                && !is_prompt_line(s)
            {
                return "idle".into();
            }
        }
        if s.starts_with("Running\u{2026}") || reading_re.is_match(s) {
            return "active".into();
        }
        // Waiting: selector cursor / numbered options with a footer hint.
        if (sl.contains("do you want") || sl.contains("would you like"))
            && (clean.contains("\u{276f} 1.") || clean.contains("1. Yes"))
        {
            return "waiting".into();
        }
        if sl.contains("esc to cancel") && (clean.contains("\u{276f} 1.") || sl.contains("enter to select")) {
            return "waiting".into();
        }
    }
    if clean.contains("\u{276f} 1.") || (clean.contains("\u{2502} \u{276f} 1.") ) {
        return "waiting".into();
    }
    if clean.contains('\u{276f}') {
        return "idle".into();
    }
    String::new()
}

/// py:18676 _clean_gemini_frame — keep only the LAST instance of each chrome
/// line class.
fn clean_gemini_frame(text: &str) -> String {
    let patterns = [
        r"^\s*workspace \(/directory\)",
        r"^\s*/model\s*$",
        r"no sandbox",
        r"^\s*Auto\s*$",
        r"^\s*YOLO Ctrl\+Y",
        r"^\s*\? for shortcuts",
        r"^\s*\d+ GEMINI\.md file",
    ];
    let lines: Vec<&str> = text.split('\n').collect();
    let plain: Vec<String> = lines.iter().map(|l| strip_ansi(l)).collect();
    let mut keep = vec![true; lines.len()];
    for p in patterns {
        let re = regex::Regex::new(p).unwrap();
        let idxs: Vec<usize> =
            plain.iter().enumerate().filter(|(_, pl)| re.is_match(pl)).map(|(i, _)| i).collect();
        for i in idxs.iter().take(idxs.len().saturating_sub(1)) {
            keep[*i] = false;
        }
    }
    lines
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, l)| *l)
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Claude-project JSONL plumbing (py:20535 _project_name, py:8166
// _iter_jsonl_tail, py:5483-5661 jsonl path resolution).
// ---------------------------------------------------------------------------

/// Claude's project-dir encoding: EVERY non-alphanumeric char becomes '-'.
fn project_name(work_dir: &str) -> String {
    let resolved = expanduser(work_dir)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| expanduser(work_dir).to_string_lossy().into_owned());
    resolved.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Parsed entries from the tail of a JSONL file (bounded read).
fn iter_jsonl_tail(path: &Path, max_bytes: u64) -> Vec<Value> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if size > max_bytes && f.seek(SeekFrom::Start(size - max_bytes)).is_err() {
        return vec![];
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if size > max_bytes {
        lines.next(); // discard partial first line
    }
    lines.filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect()
}

/// Newest JSONL for a session (py:5590 _session_jsonl_path_uncached): meta
/// conv-id first, then title match, then the single unclaimed candidate.
fn session_jsonl_path(name: &str) -> Option<PathBuf> {
    let cfg = parse_env(name);
    let wd = cfg.get_or("CC_DIR", "").trim().to_string();
    if wd.is_empty() {
        return None;
    }
    let meta = load_meta(name);
    let conv_id = meta_str(&meta, "cc_conversation_id");
    let cc_cwd = meta_str(&meta, "cc_cwd");
    if !conv_id.is_empty() {
        for base in [cc_cwd.trim(), wd.as_str()] {
            if base.is_empty() {
                continue;
            }
            let cand = claude_home().join("projects").join(project_name(base)).join(format!("{conv_id}.jsonl"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let project_dir = claude_home().join("projects").join(project_name(&wd));
    let Ok(rd) = std::fs::read_dir(&project_dir) else { return None };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    if files.is_empty() {
        return None;
    }
    if files.len() == 1 {
        return Some(files[0].1.clone());
    }
    for (_, jf) in &files {
        use std::io::BufRead;
        let Ok(f) = std::fs::File::open(jf) else { continue };
        let mut first = String::new();
        if std::io::BufReader::new(f).read_line(&mut first).is_err() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<Value>(&first) {
            if rec["customTitle"] == json!(name) || rec["sessionName"] == json!(name) {
                return Some(jf.clone());
            }
        }
    }
    // Exclude conversations claimed by SIBLING sessions; only a single
    // unclaimed candidate may be returned (shared-workdir bleed guard).
    let mut owned = std::collections::BTreeSet::new();
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("env") {
                continue;
            }
            let Some(other) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if other == name {
                continue;
            }
            let ocid = meta_str(&load_meta(other), "cc_conversation_id");
            if !ocid.is_empty() {
                owned.insert(ocid);
            }
        }
    }
    let unclaimed: Vec<&PathBuf> = files
        .iter()
        .map(|(_, p)| p)
        .filter(|p| {
            p.file_stem().and_then(|s| s.to_str()).map(|s| !owned.contains(s)).unwrap_or(true)
        })
        .collect();
    if unclaimed.len() == 1 {
        Some(unclaimed[0].clone())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Markdown → ANSI transcript renderer (py:5675-5957). Fail-safe: any panic
// risk is avoided structurally; the table renderer clamps widths.
// ---------------------------------------------------------------------------

const MD_BASE: &str = "\x1b[39m";

fn md_inline(s: &str) -> String {
    use std::sync::OnceLock;
    static CODE: OnceLock<regex::Regex> = OnceLock::new();
    static BOLD1: OnceLock<regex::Regex> = OnceLock::new();
    static BOLD2: OnceLock<regex::Regex> = OnceLock::new();
    let code = CODE.get_or_init(|| regex::Regex::new(r"`([^`\n]+)`").unwrap());
    let bold1 = BOLD1.get_or_init(|| regex::Regex::new(r"\*\*([^*\n]+?)\*\*").unwrap());
    let bold2 = BOLD2.get_or_init(|| regex::Regex::new(r"__([^_\n]+?)__").unwrap());
    let s = code.replace_all(s, format!("\x1b[38;5;153m$1{MD_BASE}").as_str());
    let s = bold1.replace_all(&s, "\x1b[1m$1\x1b[22m");
    let s = bold2.replace_all(&s, "\x1b[1m$1\x1b[22m");
    // Italic (Python uses lookarounds the regex crate lacks); the simple
    // single-star form is rare in transcripts — bold/code carry the weight.
    s.into_owned()
}

fn md_table_sep_re() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^\s*\|?\s*:?-{2,}:?\s*(?:\|\s*:?-{2,}:?\s*)+\|?\s*$").unwrap()
    })
}

fn md_render_table(block: &[&str], max_width: usize) -> String {
    fn cells(row: &str) -> Vec<String> {
        let mut r = row.trim();
        r = r.strip_prefix('|').unwrap_or(r);
        r = r.strip_suffix('|').unwrap_or(r);
        r.split('|').map(|c| c.trim().to_string()).collect()
    }
    fn strip_md(s: &str) -> String {
        let re1 = regex::Regex::new(r"`([^`]+)`").unwrap();
        let re2 = regex::Regex::new(r"\*\*([^*]+)\*\*").unwrap();
        let re3 = regex::Regex::new(r"__([^_]+)__").unwrap();
        let s = re1.replace_all(s, "$1");
        let s = re2.replace_all(&s, "$1");
        re3.replace_all(&s, "$1").into_owned()
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    rows.push(cells(block[0]).iter().map(|c| strip_md(c)).collect());
    for r in &block[2..] {
        rows.push(cells(r).iter().map(|c| strip_md(c)).collect());
    }
    let ncol = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncol == 0 {
        return block.join("\n");
    }
    for r in rows.iter_mut() {
        r.resize(ncol, String::new());
    }
    let natural: Vec<usize> = (0..ncol)
        .map(|ci| rows.iter().map(|r| r[ci].chars().count()).max().unwrap_or(0))
        .collect();
    let avail = std::cmp::max(ncol * 6, max_width.saturating_sub(3 * ncol + 1));
    let mut widths = natural.clone();
    let mut guard = 0;
    while widths.iter().sum::<usize>() > avail && guard < 10000 {
        guard += 1;
        let mx = (0..ncol).max_by_key(|c| widths[*c]).unwrap();
        if widths[mx] <= 6 {
            break;
        }
        widths[mx] -= 1;
    }
    for w in widths.iter_mut() {
        if *w == 0 {
            *w = 1;
        }
    }
    fn wrap_cell(text: &str, w: usize) -> Vec<String> {
        if text.is_empty() {
            return vec![String::new()];
        }
        let mut out = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let wl = word.chars().count();
            if line.is_empty() {
                if wl <= w {
                    line = word.to_string();
                } else {
                    for chunk in word.chars().collect::<Vec<_>>().chunks(w) {
                        out.push(chunk.iter().collect());
                    }
                }
            } else if line.chars().count() + 1 + wl <= w {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                if wl <= w {
                    line = word.to_string();
                } else {
                    for chunk in word.chars().collect::<Vec<_>>().chunks(w) {
                        out.push(chunk.iter().collect());
                    }
                }
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
        if out.is_empty() {
            out.push(String::new());
        }
        out
    }
    let render_row = |cs: &[String], header: bool| -> String {
        let wrapped: Vec<Vec<String>> = (0..ncol).map(|ci| wrap_cell(&cs[ci], widths[ci])).collect();
        let h = wrapped.iter().map(|w| w.len()).max().unwrap_or(1);
        let mut out = Vec::new();
        for k in 0..h {
            let mut parts = Vec::new();
            for (ci, cellw) in wrapped.iter().enumerate() {
                let seg = cellw.get(k).cloned().unwrap_or_default();
                let pad = format!("{}{}", seg, " ".repeat(widths[ci].saturating_sub(seg.chars().count())));
                if header && !seg.is_empty() {
                    parts.push(format!("\x1b[1m{pad}\x1b[22m"));
                } else {
                    parts.push(pad);
                }
            }
            out.push(format!("\u{2502} {} \u{2502}", parts.join(" \u{2502} ")));
        }
        out.join("\n")
    };
    let bar = |l: char, m: char, r: char| -> String {
        let mid: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(w + 2)).collect();
        format!("{l}{}{r}", mid.join(&m.to_string()))
    };
    let mut res = vec![bar('\u{250c}', '\u{252c}', '\u{2510}'), render_row(&rows[0], true), bar('\u{251c}', '\u{253c}', '\u{2524}')];
    for r in &rows[1..] {
        res.push(render_row(r, false));
    }
    res.push(bar('\u{2514}', '\u{2534}', '\u{2518}'));
    res.join("\n")
}

fn md_to_ansi(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let header_re = regex::Regex::new(r"^(#{1,6})\s+(.*)$").unwrap();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let ln = lines[i];
        if ln.trim().starts_with('|')
            && ln.matches('|').count() >= 2
            && i + 1 < lines.len()
            && md_table_sep_re().is_match(lines[i + 1])
        {
            let mut blk = vec![ln, lines[i + 1]];
            let mut j = i + 2;
            while j < lines.len() && lines[j].trim().starts_with('|') && lines[j].matches('|').count() >= 2 {
                blk.push(lines[j]);
                j += 1;
            }
            out.push(md_render_table(&blk, 100));
            i = j;
            continue;
        }
        if let Some(c) = header_re.captures(ln) {
            out.push(format!("\x1b[1m{}\x1b[22m", md_inline(&c[2])));
            i += 1;
            continue;
        }
        out.push(md_inline(ln));
        i += 1;
    }
    out.join("\n")
}

fn user_echo_ansi(txt: &str) -> String {
    format!(
        "\x1b[38;5;239m\x1b[48;5;237m\u{276f} \x1b[38;5;231m{}\x1b[39m\x1b[49m",
        txt.replace('\n', "\n  ")
    )
}

fn tool_brief(inp: &Value) -> String {
    let Some(obj) = inp.as_object() else { return String::new() };
    for k in ["command", "file_path", "path", "pattern", "query", "url", "prompt", "description", "old_string"] {
        if let Some(v) = obj.get(k).and_then(|v| v.as_str()) {
            let v = v.replace('\n', " ").trim().to_string();
            if !v.is_empty() {
                let t = chars_truncate(&v, 90);
                return if v.chars().count() > 90 { format!("{t}\u{2026}") } else { t };
            }
        }
    }
    String::new()
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|x| {
                if let Some(s) = x.as_str() {
                    Some(s.to_string())
                } else if x["type"] == json!("text") {
                    x["text"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string(),
    }
}

/// py:5833 _render_session_transcript — clean ANSI render of the JSONL tail.
fn render_session_transcript(name: &str, max_chars: usize) -> String {
    let Some(path) = session_jsonl_path(name) else { return String::new() };
    let max_read = std::cmp::max(max_chars * 5, 5_000_000) as u64;
    let mut out: Vec<String> = Vec::new();
    let sysrem = regex::Regex::new(r"(?s)<system-reminder>.*?</system-reminder>").unwrap();
    let tasknote = regex::Regex::new(r"(?s)<task-notification>.*?</task-notification>").unwrap();
    let caveat = regex::Regex::new(r"(?s)<local-command-caveat>.*?</local-command-caveat>").unwrap();
    let cmd_re = regex::Regex::new(r"(?s)<command-name>(.*?)</command-name>").unwrap();
    let arg_re = regex::Regex::new(r"(?s)<command-args>(.*?)</command-args>").unwrap();
    let out_re = regex::Regex::new(r"(?s)<local-command-stdout>(.*?)</local-command-stdout>").unwrap();
    for o in iter_jsonl_tail(&path, max_read) {
        let t = o["type"].as_str().unwrap_or("");
        if t != "user" && t != "assistant" {
            continue;
        }
        let Some(msg) = o.get("message").and_then(|m| m.as_object()) else { continue };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let blocks: Vec<Value> = match msg.get("content") {
            Some(Value::String(s)) => vec![json!({"type": "text", "text": s})],
            Some(Value::Array(a)) => a.clone(),
            _ => continue,
        };
        for b in blocks {
            let bt = b["type"].as_str().unwrap_or("");
            match bt {
                "text" => {
                    let mut txt = b["text"].as_str().unwrap_or("").trim().to_string();
                    if txt.is_empty() {
                        continue;
                    }
                    if role == "user" {
                        if txt.contains("<system-reminder>")
                            || txt.contains("<task-notification>")
                            || txt.contains("<local-command-caveat>")
                        {
                            txt = sysrem.replace_all(&txt, "").into_owned();
                            txt = tasknote.replace_all(&txt, "").into_owned();
                            txt = caveat.replace_all(&txt, "").into_owned();
                            txt = txt.trim().to_string();
                            if txt.is_empty() {
                                continue;
                            }
                        }
                        let m_cmd = cmd_re.captures(&txt);
                        let m_out = out_re.captures(&txt);
                        if m_cmd.is_some() || m_out.is_some() {
                            if let Some(mc) = &m_cmd {
                                let mut cmd_line = mc[1].trim().to_string();
                                if !cmd_line.is_empty() {
                                    if let Some(ma) = arg_re.captures(&txt) {
                                        let a = ma[1].trim();
                                        if !a.is_empty() {
                                            cmd_line = format!("{cmd_line} {a}");
                                        }
                                    }
                                    out.push(user_echo_ansi(&cmd_line));
                                }
                            }
                            if let Some(mo) = &m_out {
                                let body = mo[1].trim();
                                if !body.is_empty() {
                                    for (k, ln) in body.split('\n').take(6).enumerate() {
                                        let prefix = if k == 0 { "  \u{23bf}  " } else { "     " };
                                        out.push(format!("\x1b[38;5;246m{}{}\x1b[0m", prefix, ln.trim_end()));
                                    }
                                }
                            }
                            out.push(String::new());
                            continue;
                        }
                        out.push(user_echo_ansi(&txt));
                    } else {
                        let body = md_to_ansi(&txt).replace('\n', "\n  ");
                        out.push(format!("\x1b[38;5;231m\u{23fa}\x1b[39m {body}\x1b[0m"));
                    }
                    out.push(String::new());
                }
                "tool_use" => {
                    let nm = b["name"].as_str().unwrap_or("tool");
                    let arg = tool_brief(&b["input"]);
                    let suffix = if arg.is_empty() { String::new() } else { format!("({arg})") };
                    out.push(format!("\x1b[38;5;114m\u{23fa}\x1b[39m \x1b[1m{nm}\x1b[0m{suffix}"));
                }
                "tool_result" => {
                    let raw = tool_result_text(&b["content"]);
                    let mut rlines: Vec<&str> = raw.split('\n').map(|l| l.trim_end()).collect();
                    while rlines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
                        rlines.remove(0);
                    }
                    while rlines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                        rlines.pop();
                    }
                    if !rlines.is_empty() {
                        const MAXL: usize = 6;
                        const MAXW: usize = 200;
                        for (k, ln) in rlines.iter().take(MAXL).enumerate() {
                            let mut ln = (*ln).to_string();
                            if ln.chars().count() > MAXW {
                                ln = format!("{}\u{2026}", chars_truncate(&ln, MAXW));
                            }
                            let prefix = if k == 0 { "  \u{23bf}  " } else { "     " };
                            out.push(format!("\x1b[38;5;246m{prefix}{ln}\x1b[0m"));
                        }
                        if rlines.len() > MAXL {
                            let extra = rlines.len() - MAXL;
                            let word = if extra != 1 { " more lines" } else { " more line" };
                            out.push(format!("\x1b[38;5;246m     \u{2026} +{extra}{word}\x1b[0m"));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut text = out.join("\n").trim_matches('\n').to_string();
    if text.chars().count() > max_chars {
        let chars: Vec<char> = text.chars().collect();
        text = chars[chars.len() - max_chars..].iter().collect();
        if let Some(nl) = text.find('\n') {
            if nl > 0 {
                text = text[nl + 1..].to_string();
            }
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Flag-string helpers (py:22390-22614): shlex-equivalent split/quote, model /
// effort / yolo manipulation. split_flags errs on unbalanced quotes exactly
// where Python's shlex raises ValueError (the "don't wipe the user's flags"
// contract).
// ---------------------------------------------------------------------------

const SESSION_PROVIDERS: [&str; 4] = ["claude", "codex", "gemini", "iterm2"];
const PROVIDER_YOLO_FLAGS: [&str; 3] = [
    "--dangerously-skip-permissions",
    "--dangerously-bypass-approvals-and-sandbox",
    "--yolo",
];
const VALID_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn split_flags(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(x) => cur.push(x),
                        None => return Err("No closing quotation".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(x @ ('"' | '\\' | '$' | '`')) => cur.push(x),
                            Some(x) => {
                                cur.push('\\');
                                cur.push(x);
                            }
                            None => return Err("No closing quotation".into()),
                        },
                        Some(x) => cur.push(x),
                        None => return Err("No closing quotation".into()),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(x) => {
                    in_word = true;
                    cur.push(x);
                }
                None => return Err("No escaped character".into()),
            },
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// POSIX single-quote escaping (shlex.quote parity).
fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b'=' | b':' | b'@' | b'%' | b'+' | b',')
        })
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn shell_quote_flags(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    match split_flags(s) {
        Ok(tokens) => tokens.iter().map(|t| sh_quote(t)).collect::<Vec<_>>().join(" "),
        Err(_) => sh_quote(s),
    }
}

fn strip_token_from_flags(flags: &str, flag: &str) -> Result<String, String> {
    if flags.is_empty() {
        return Ok(String::new());
    }
    let tokens = split_flags(flags)?;
    let eq_form = format!("{flag}=");
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == flag && i + 1 < tokens.len() {
            i += 2;
            continue;
        }
        if t.starts_with(&eq_form) {
            i += 1;
            continue;
        }
        filtered.push(t.clone());
        i += 1;
    }
    Ok(filtered.iter().map(|t| sh_quote(t)).collect::<Vec<_>>().join(" "))
}

fn strip_model_from_flags(flags: &str) -> Result<String, String> {
    strip_token_from_flags(flags, "--model")
}

fn extract_model_from_flags(flags: &str) -> String {
    let Ok(tokens) = split_flags(flags) else { return String::new() };
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "--model" && i + 1 < tokens.len() {
            return tokens[i + 1].clone();
        }
        if let Some(v) = tokens[i].strip_prefix("--model=") {
            return v.to_string();
        }
        i += 1;
    }
    String::new()
}

const MODEL_ID_MAX_LEN: usize = 100;

fn validate_model_name(value: &Value) -> Result<String, String> {
    let Some(s) = value.as_str() else { return Err("model must be a string".into()) };
    let normalized = s.trim().to_string();
    if normalized.chars().count() > MODEL_ID_MAX_LEN {
        return Err(format!("model name too long (max {MODEL_ID_MAX_LEN} chars)"));
    }
    let re = regex::Regex::new(r"^[A-Za-z0-9._:\[\]@/+][A-Za-z0-9._:\[\]@/+\-]*$").unwrap();
    if !normalized.is_empty() && !re.is_match(&normalized) {
        return Err("invalid model name (allowed: alphanumeric and ._:[]@/+-, no leading hyphen)".into());
    }
    Ok(normalized)
}

fn validate_effort(value: &Value) -> Result<String, String> {
    let Some(s) = value.as_str() else { return Err("effort must be a string".into()) };
    let normalized = s.trim().to_lowercase();
    if !normalized.is_empty() && !VALID_EFFORTS.contains(&normalized.as_str()) {
        return Err(format!("invalid effort (allowed: {})", VALID_EFFORTS.join(", ")));
    }
    Ok(normalized)
}

fn set_effort_flag(flags: &str, effort: &str) -> Result<String, String> {
    let base = strip_token_from_flags(flags, "--effort")?;
    if effort.is_empty() {
        return Ok(base);
    }
    Ok(if base.is_empty() { format!("--effort {effort}") } else { format!("{base} --effort {effort}") })
}

fn provider_yolo_flag(provider: &str) -> &'static str {
    match provider {
        "codex" => "--dangerously-bypass-approvals-and-sandbox",
        "gemini" => "--yolo",
        _ => "--dangerously-skip-permissions",
    }
}

fn strip_provider_yolo_flags(flags: &str) -> String {
    if flags.is_empty() {
        return String::new();
    }
    let Ok(tokens) = split_flags(flags) else {
        let mut out = flags.to_string();
        for f in PROVIDER_YOLO_FLAGS {
            out = out.replace(f, "");
        }
        let re = regex::Regex::new(r"--approval-mode(?:=|\s+)yolo\b").unwrap();
        return re.replace_all(&out, "").trim().to_string();
    };
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if PROVIDER_YOLO_FLAGS.contains(&t.as_str()) {
            i += 1;
            continue;
        }
        if t == "--approval-mode" && i + 1 < tokens.len() && tokens[i + 1] == "yolo" {
            i += 2;
            continue;
        }
        if t == "--approval-mode=yolo" {
            i += 1;
            continue;
        }
        filtered.push(t.clone());
        i += 1;
    }
    filtered.iter().map(|t| sh_quote(t)).collect::<Vec<_>>().join(" ")
}

fn is_yolo_enabled(flags: &str, cfg: &EnvFile) -> bool {
    PROVIDER_YOLO_FLAGS.iter().any(|f| flags.contains(f))
        || flags.contains("--approval-mode=yolo")
        || flags.contains("--approval-mode yolo")
        || matches!(cfg.get("CC_AUTO_CONTINUE"), Some("1" | "true" | "yes"))
}

fn default_model_for_provider(provider: &str) -> String {
    match provider {
        "codex" => "gpt-5.5".into(),
        "gemini" => "auto".into(),
        _ => get_default_model(),
    }
}

fn provider_label(provider: &str) -> &str {
    match provider {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "iterm2" => "iTerm2",
        other => {
            if other.is_empty() {
                "Claude Code"
            } else {
                other
            }
        }
    }
}

fn get_default_model() -> String {
    let defaults = EnvFile::load(&home().join("defaults.env"));
    let m = extract_model_from_flags(defaults.get_or("CC_DEFAULT_FLAGS", ""));
    if m.is_empty() { "sonnet".into() } else { m }
}

// ---------------------------------------------------------------------------
// Shared-DB helpers. All writes ride the store's writer thread; every table
// is CREATEd IF NOT EXISTS first so a fresh Rust-only AMUX_HOME (unit tests)
// works — on the live shared DB these are no-ops against Python's schema.
// ---------------------------------------------------------------------------

fn ensure_fleet_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT, ts REAL NOT NULL,
            session TEXT NOT NULL DEFAULT '', type TEXT NOT NULL,
            data TEXT, idem TEXT, source TEXT NOT NULL DEFAULT '');
         CREATE UNIQUE INDEX IF NOT EXISTS idx_sev_idem ON session_events(idem) WHERE idem IS NOT NULL;
         CREATE TABLE IF NOT EXISTS steering_queue (
            id TEXT PRIMARY KEY, session TEXT NOT NULL, text TEXT NOT NULL,
            queued_at REAL NOT NULL, guard TEXT);
         CREATE TABLE IF NOT EXISTS steering_history (
            id TEXT PRIMARY KEY, session TEXT NOT NULL, text TEXT NOT NULL,
            queued_at REAL, delivered_at REAL NOT NULL);
         CREATE TABLE IF NOT EXISTS share_tokens (
            token TEXT PRIMARY KEY, session TEXT NOT NULL,
            perms TEXT NOT NULL DEFAULT 'output', created_at INTEGER NOT NULL,
            expires_at INTEGER, label TEXT NOT NULL DEFAULT '');
         CREATE TABLE IF NOT EXISTS send_dedup (
            session TEXT NOT NULL, msg_id TEXT NOT NULL, ts INTEGER NOT NULL,
            PRIMARY KEY (session, msg_id));
         CREATE TABLE IF NOT EXISTS cmd_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL,
            type TEXT NOT NULL DEFAULT 'direct', session TEXT NOT NULL DEFAULT '',
            ts INTEGER NOT NULL, origin TEXT NOT NULL DEFAULT '');
         CREATE TABLE IF NOT EXISTS prefs (key TEXT PRIMARY KEY, value TEXT);",
    )?;
    // Python's steering_queue predates `guard` and gained it via ALTER; a DB
    // created by Python's schema block lacks it. Add-if-missing, ignore
    // "duplicate column".
    let _ = conn.execute("ALTER TABLE steering_queue ADD COLUMN guard TEXT", []);
    let _ = conn.execute("ALTER TABLE cmd_history ADD COLUMN origin TEXT NOT NULL DEFAULT ''", []);
    Ok(())
}

fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
fn now_i64() -> i64 {
    now_f64() as i64
}

/// py:7593 _emit_event — append-only, idempotent on `idem`, never fails the
/// caller.
async fn emit_event(state: &AppState, session: &str, etype: &str, data: Option<Value>, idem: Option<String>, source: &str) {
    let session = session.to_string();
    let etype = etype.to_string();
    let source = source.to_string();
    let _ = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute(
                "INSERT OR IGNORE INTO session_events (ts, session, type, data, idem, source) VALUES (?,?,?,?,?,?)",
                rusqlite::params![
                    now_f64(),
                    session,
                    etype,
                    data.map(|d| d.to_string()),
                    idem,
                    source
                ],
            )?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
}

/// The secret-redaction pass Python applies before any chat text lands in a
/// DB row (py:8676 _cmd_hist_record / py:8655 steer history — AMUX-2525).
/// Same pattern family as the pipe-pane redactor (py:21478).
fn redact_secrets(text: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"((?:mxp|usr|ret)_sk)_[A-Za-z0-9_-]+|((?:AMUX_MIXPEEK_OPS_TOKEN|ANTHROPIC_API_KEY|OPENAI_API_KEY|GOOGLE_MAPS_API_KEY|GOOGLE_API_KEY|CLOUDFLARE_API_TOKEN|ELEVENLABS_API_KEY|POSTHOG_KEY|POSTHOG_PERSONAL_API_KEY)=)[^\s\r\n]+|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]+|sk-ant-[A-Za-z0-9_-]+|sk-proj-[A-Za-z0-9_-]+|sk[_-][A-Za-z0-9]{32,}|AIza[0-9A-Za-z_-]{30,}|(?:phx|phc)_[A-Za-z0-9]+",
        )
        .expect("redact regex")
    });
    re.replace_all(text, |caps: &regex::Captures| {
        if let Some(p) = caps.get(1) {
            format!("{}_REDACTED", p.as_str())
        } else if let Some(p) = caps.get(2) {
            format!("{}REDACTED", p.as_str())
        } else {
            "SECRET_REDACTED".to_string()
        }
    })
    .into_owned()
}

const CMD_HIST_KEEP: i64 = 200;

/// py:8676 _cmd_hist_record — Messages history, origin-tagged, pruned.
async fn cmd_hist_record(state: &AppState, session: &str, text: &str, ctype: &str, origin: &str) {
    if session.is_empty() || text.is_empty() {
        return;
    }
    let session = session.to_string();
    let text = redact_secrets(text);
    let ctype = ctype.to_string();
    let origin: String = origin.chars().take(80).collect();
    let _ = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute(
                "INSERT INTO cmd_history (text, type, session, ts, origin) VALUES (?,?,?,?,?)",
                rusqlite::params![text, ctype, session, now_i64() * 1000, origin],
            )?;
            conn.execute(
                "DELETE FROM cmd_history WHERE session=?1 AND id NOT IN \
                 (SELECT id FROM cmd_history WHERE session=?1 ORDER BY ts DESC LIMIT ?2)",
                rusqlite::params![session, CMD_HIST_KEEP],
            )?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
}

/// py:8595 _steer_enqueue — durable queue row + message.queued event.
/// Dedup-on-enqueue: identical text (or same guard) replaces, never stacks.
async fn steer_enqueue(state: &AppState, name: &str, text: &str, guard: &str) -> String {
    let msg_id = format!("steer-{}", (now_f64() * 1000.0) as i64);
    let id = msg_id.clone();
    let session = name.to_string();
    let text_s = text.to_string();
    let guard_s = guard.to_string();
    let _ = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute(
                "DELETE FROM steering_queue WHERE session=?1 AND (text=?2 OR (?3 != '' AND guard=?3))",
                rusqlite::params![session, text_s, guard_s],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO steering_queue(id, session, text, queued_at, guard) VALUES(?,?,?,?,?)",
                rusqlite::params![
                    id,
                    session,
                    text_s,
                    now_f64(),
                    if guard_s.is_empty() { None } else { Some(guard_s.clone()) }
                ],
            )?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
    emit_event(
        state,
        name,
        "message.queued",
        Some(json!({"chars": text.chars().count(), "preview": chars_truncate(text, 120), "guard": if guard.is_empty() { Value::Null } else { json!(guard) }})),
        Some(format!("q:{msg_id}")),
        "steering",
    )
    .await;
    msg_id
}

/// py:25236 _send_dedup_seen — idempotency across client retries, persisted
/// because the loss window IS a server restart.
async fn send_dedup_seen(state: &AppState, name: &str, msg_id: &str) -> bool {
    let session = name.to_string();
    let msg_id = msg_id.to_string();
    let reply = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute("DELETE FROM send_dedup WHERE ts < ?", [now_i64() - 600])?;
            let dup = conn
                .execute(
                    "INSERT INTO send_dedup (session, msg_id, ts) VALUES (?,?,?)",
                    rusqlite::params![session, msg_id, now_i64()],
                )
                .is_err();
            Ok(crate::db::WriteOutcome {
                applied: !dup,
                events: vec![],
            })
        })
        .await;
    match reply {
        Ok(r) => !r.applied,
        Err(_) => false, // dedup is best-effort; never block a send on it
    }
}

async fn send_dedup_forget(state: &AppState, name: &str, msg_id: &str) {
    let session = name.to_string();
    let msg_id = msg_id.to_string();
    let _ = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute(
                "DELETE FROM send_dedup WHERE session=? AND msg_id=?",
                rusqlite::params![session, msg_id],
            )?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
}

// ---------------------------------------------------------------------------
// is_running (py:4372): tmux session present + not a bare shell + the pane
// shell has a child. herdr: agent presence. iTerm2: unsupported (501 at the
// verb layer; here it reads not-running).
// ---------------------------------------------------------------------------

async fn is_running(name: &str) -> bool {
    let cfg = parse_env(name);
    if !iterm2_id(&cfg).is_empty() {
        return false;
    }
    if backend_of_cfg(&cfg) == "herdr" {
        return herdr_agent_running(name).await;
    }
    let tmux_sess = tmux_name(name);
    if !tmux_sessions_set().await.contains(&tmux_sess) {
        return false;
    }
    let output = tmux_capture(name, 10).await;
    if output.is_empty() {
        return true;
    }
    if at_shell_prompt(&output) {
        return false;
    }
    // Shell alive but childless == claude gone even without a visible prompt.
    let stq = st(name);
    if let Some(out) = tmux(&["list-panes", "-t", &stq, "-F", "#{pane_pid}"]).await {
        if out.status.success() {
            let pid = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
            if !pid.is_empty() {
                if let Some(ch) = run_cmd("pgrep", &["-P", &pid], OP_TIMEOUT).await {
                    if ch.stdout.iter().all(|b| b.is_ascii_whitespace()) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// send_text (py:25432) — the delivery choreography. Ported: restart guard,
// resume-picker guard, auto-wake, boot-in-flight wait, status-gated Escape
// discipline (the 1.3s double-Escape rule), C-u clear, paste-buffer for >400
// chars, @/slash picker handling, steering enqueue for waiting selectors.
// NOT ported: _verify_submitted's JSONL evidence gate (gap named in the
// module doc) — "sent" here means the keys landed and Enter was pressed.
// ---------------------------------------------------------------------------

fn at_picker_text(text: &str) -> bool {
    // Python's _AT_PICKER_RE: text that opens Claude's autocomplete picker.
    text.contains('@') || text.trim_start().starts_with('/')
}

async fn send_after_ready(state: AppState, name: String, text: String, timeout_s: u64) {
    // py:24889 _send_after_ready — wait for Claude's input prompt, then send.
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s);
    while std::time::Instant::now() < deadline {
        let out = tmux_capture(&name, 15).await;
        if !out.is_empty() {
            let clean = strip_ansi(&out);
            if claude_ui_visible(&clean) && !at_resume_picker(&clean) {
                sleep_ms(1200).await;
                let _ = send_text_boxed(&state, &name, &text, false).await;
                return;
            }
        }
        sleep_ms(500).await;
    }
}

async fn send_text(state: &AppState, name: &str, text: &str, defer_if_busy: bool) -> (bool, String) {
    send_text_inner(state, name, text, defer_if_busy, false).await
}

fn send_text_boxed<'a>(
    state: &'a AppState,
    name: &'a str,
    text: &'a str,
    defer_if_busy: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = (bool, String)> + Send + 'a>> {
    Box::pin(send_text_inner(state, name, text, defer_if_busy, false))
}

async fn send_text_inner(
    state: &AppState,
    name: &str,
    text: &str,
    defer_if_busy: bool,
    from_steering: bool,
) -> (bool, String) {
    let cfg = parse_env(name);
    if !iterm2_id(&cfg).is_empty() {
        return (false, "iTerm2-backed sessions are not supported by the rust origin yet".into());
    }
    if backend_of_cfg(&cfg) == "herdr" {
        return herdr_send(name, text).await;
    }
    let boot_in_flight = {
        let meta = load_meta(name);
        let last_started = meta.get("last_started").and_then(|v| v.as_i64()).unwrap_or(0);
        now_i64() - last_started < 20
    };
    let out_st = tmux_capture(name, 15).await;
    if !out_st.is_empty() && at_resume_picker(&strip_ansi(&out_st)) {
        return (false, "session is in resume picker".into());
    }
    let mut needs_wake = false;
    if !out_st.is_empty() && at_shell_prompt(&strip_ansi(&out_st)) {
        needs_wake = true; // terminal visible but Claude has exited
    } else if !is_running(name).await {
        needs_wake = true;
    }
    if needs_wake {
        if boot_in_flight {
            let st2 = state.clone();
            let (n, t) = (name.to_string(), text.to_string());
            tokio::spawn(async move { send_after_ready(st2, n, t, 30).await });
            return (true, "sent (waiting for in-flight boot)".into());
        }
        if !env_path(name).exists() {
            return (false, "not running".into());
        }
        // Auto-wake parity (py:25463): start, then deliver once ready.
        let (ok, msg) = start_session(state, name, "", false).await;
        if !ok {
            return (false, format!("auto-wake failed: {msg}"));
        }
        let st2 = state.clone();
        let (n, t) = (name.to_string(), text.to_string());
        tokio::spawn(async move { send_after_ready(st2, n, t, 60).await });
        return (true, "sent (auto-woke)".into());
    }
    let mut text = text.to_string();
    if text.is_empty() {
        // Suggested-prompt extraction (py:25501): pull the ❯ suggestion.
        let pane = tmux_capture(name, 0).await;
        let clean = strip_ansi(&pane);
        let nonblank: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
        let footer: Vec<&str> = nonblank[nonblank.len().saturating_sub(4)..]
            .iter()
            .filter(|l| !l.trim_start().starts_with('\u{276f}'))
            .copied()
            .collect();
        if footer.iter().any(|l| {
            let ll = l.to_lowercase();
            ll.contains("to navigate") || ll.contains("enter to select")
        }) {
            return (true, "no suggestion found".into());
        }
        for line in clean.lines().rev() {
            let line = line.trim();
            if line.starts_with('\u{276f}') || line.starts_with('>') {
                let suggested = line.trim_start_matches(['\u{276f}', '>', '\u{a0}', ' ']).trim();
                if !suggested.is_empty() {
                    text = suggested.to_string();
                    break;
                }
            }
        }
        if text.is_empty() {
            return (true, "no suggestion found".into());
        }
    }
    let status = detect_claude_status(&tmux_capture(name, 12).await);
    let mut generating = status == "active";
    let waiting = status == "waiting";
    if defer_if_busy && waiting {
        // A live selector parks an automated send (py:25545; the
        // AskUserQuestion kill of 2026-07-15).
        steer_enqueue(state, name, &text, "").await;
        return (true, "queued (steering) — session at a selector, delivers when it resolves".into());
    }
    if waiting && from_steering {
        return (false, "session at a selector — retry at next idle boundary".into());
    }
    if generating && at_picker_text(&text) {
        if from_steering {
            return (false, "session started generating — retry at next turn boundary".into());
        }
        steer_enqueue(state, name, &text, "").await;
        return (true, "queued (steering) until turn end — @/slash needs the picker closed".into());
    }
    let mut esc_at: Option<std::time::Instant> = None;
    if !generating {
        // Fresh re-check right before the Escape (py:25597): "esc to
        // interrupt" in the pane is the reliable generating signal.
        if tmux_capture(name, 12).await.to_lowercase().contains("esc to interrupt") {
            generating = true;
            if from_steering {
                return (false, "session started generating — retry at next turn boundary".into());
            }
        } else if !waiting {
            send_key(name, "Escape").await;
            esc_at = Some(std::time::Instant::now());
            sleep_ms(50).await;
        }
    }
    send_key(name, "C-u").await;
    sleep_ms(40).await;
    if text.chars().count() > 400 {
        // Named tmux buffer + paste-buffer -p (py:25630).
        let buf_name = format!("amux-{}-{}", name, (now_f64() * 1000.0) as i64);
        let tmp = std::env::temp_dir().join(format!("{buf_name}.txt"));
        if std::fs::write(&tmp, &text).is_err() {
            return (false, "could not stage paste buffer".into());
        }
        let tmp_s = tmp.to_string_lossy().into_owned();
        let ptq = pt(name);
        let ok1 = matches!(tmux(&["load-buffer", "-b", &buf_name, &tmp_s]).await, Some(o) if o.status.success());
        let ok2 = ok1
            && matches!(
                tmux(&["paste-buffer", "-p", "-b", &buf_name, "-t", &ptq]).await,
                Some(o) if o.status.success()
            );
        let _ = tmux(&["delete-buffer", "-b", &buf_name]).await;
        let _ = std::fs::remove_file(&tmp);
        if !ok2 {
            return (false, "paste-buffer failed".into());
        }
    } else if !send_literal(name, &text).await {
        return (false, "send-keys failed".into());
    }
    sleep_ms(20).await;
    if !generating && at_picker_text(&text) {
        // Close the autocomplete picker so Enter submits (py:25655), spaced
        // ≥1.3s from the leading Escape — a closer pair eats the message.
        if let Some(at) = esc_at {
            let elapsed = at.elapsed();
            if elapsed < Duration::from_millis(1300) {
                tokio::time::sleep(Duration::from_millis(1300) - elapsed).await;
            }
        }
        send_key(name, "Escape").await;
        sleep_ms(60).await;
    }
    send_key(name, "Enter").await;
    if generating {
        return (true, "sent (queued while generating)".into());
    }
    (true, "sent".into())
}

/// py:25815 send_keys — allowed control keys only.
const ALLOWED_TMUX_KEYS: [&str; 47] = [
    "Enter", "Escape", "Tab", "BTab", "Space", "BSpace", "Up", "Down", "Left", "Right", "Home",
    "End", "PageUp", "PageDown", "IC", "DC", "C-c", "C-d", "C-z", "C-l", "C-a", "C-e", "C-k",
    "C-u", "C-r", "C-p", "C-n", "C-b", "C-f", "C-w", "C-o", "C-x", "M-b", "M-f", "M-d", "F1",
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];
const ALLOWED_TMUX_CHAR_KEYS: [&str; 4] = ["y", "n", "q", "x"];

async fn send_keys_op(name: &str, keys: &str) -> (bool, String) {
    if !is_running(name).await {
        return (false, "not running".into());
    }
    if !ALLOWED_TMUX_KEYS.contains(&keys) && !ALLOWED_TMUX_CHAR_KEYS.contains(&keys) {
        return (false, format!("key '{keys}' not in allowed set"));
    }
    let ptq = pt(name);
    match tmux(&["send-keys", "-t", &ptq, keys]).await {
        Some(o) if o.status.success() => (true, "sent".into()),
        Some(o) => (false, String::from_utf8_lossy(&o.stderr).into_owned()),
        None => (false, "timeout sending keys".into()),
    }
}

// ---------------------------------------------------------------------------
// start_session (py:24218) — the launch choreography. Claude path faithful
// (resume via cc_conversation_id/cc_session_name-less UUID, --name fresh
// start, MCP registry, profile sourcing, HISTFILE, pipe-pane logging, the
// resume-picker/fresh-retry fallback). codex/gemini: command construction
// ported minus provider trust/memory side effects (gaps named). herdr: 501.
// ---------------------------------------------------------------------------

fn mcp_registry_path() -> Option<PathBuf> {
    // py:24151 — ~/.amux/mcp.json, seeded by Python from the repo. The rust
    // origin only CONSUMES an existing registry; seeding stays Python's.
    let p = home().join("mcp.json");
    if p.exists() { Some(p) } else { None }
}

fn user_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
}

fn tmux_cols() -> String {
    std::env::var("AMUX_TMUX_COLS").ok().filter(|v| v.parse::<u32>().is_ok()).unwrap_or_else(|| "220".into())
}
fn tmux_rows() -> String {
    std::env::var("AMUX_TMUX_ROWS").ok().filter(|v| v.parse::<u32>().is_ok()).unwrap_or_else(|| "50".into())
}

/// py:21478 _log_pipe_command — pipe-pane through the secret redactor.
fn log_pipe_command(log_path: &Path) -> String {
    let redactor = concat!(
        "import re,sys\n",
        "pat=re.compile(rb'((?:mxp|usr|ret)_sk)_[A-Za-z0-9_-]+|((?:AMUX_MIXPEEK_OPS_TOKEN|ANTHROPIC_API_KEY|OPENAI_API_KEY|GOOGLE_MAPS_API_KEY|GOOGLE_API_KEY|CLOUDFLARE_API_TOKEN|ELEVENLABS_API_KEY|POSTHOG_KEY|POSTHOG_PERSONAL_API_KEY)=)[^\\s\\r\\n]+|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]+|sk-ant-[A-Za-z0-9_-]+|sk-proj-[A-Za-z0-9_-]+|sk[_-][A-Za-z0-9]{32,}|AIza[0-9A-Za-z_-]{30,}|(?:phx|phc)_[A-Za-z0-9]+')\n",
        "def repl(m):\n",
        "    if m.group(1): return m.group(1)+b'_REDACTED'\n",
        "    if m.group(2): return m.group(2)+b'REDACTED'\n",
        "    return b'SECRET_REDACTED'\n",
        "for line in sys.stdin.buffer:\n",
        "    sys.stdout.buffer.write(pat.sub(repl, line))\n",
        "    sys.stdout.buffer.flush()\n",
    );
    format!("python3 -c {} >> {}", sh_quote(redactor), sh_quote(&log_path.to_string_lossy()))
}

async fn poll_shell_prompt(name: &str, timeout_ms: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        let out = tmux_capture(name, 5).await;
        if !out.is_empty() && at_shell_prompt(&strip_ansi(&out)) {
            return true;
        }
        sleep_ms(150).await;
    }
    false
}

async fn type_line(name: &str, line: &str) {
    let _ = send_literal(name, line).await;
    sleep_ms(100).await;
    send_key(name, "Enter").await;
}

fn build_claude_cmd(cfg: &EnvFile, flags: &str, default_flags: &str, session_flag: &str, extra_flags: &str) -> String {
    let custom = std::env::var("AMUX_CLAUDE_CMD").unwrap_or_default().trim().to_string();
    let mut cmd = if custom.is_empty() { "claude".to_string() } else { custom };
    if !default_flags.is_empty() {
        cmd = format!("{cmd} {}", shell_quote_flags(default_flags));
    }
    if !flags.is_empty() {
        cmd = format!("{cmd} {}", shell_quote_flags(flags));
    }
    if !session_flag.is_empty() {
        cmd = format!("{cmd} {session_flag}");
    }
    if !extra_flags.is_empty() {
        cmd = format!("{cmd} {}", shell_quote_flags(extra_flags));
    }
    let mcp_val = cfg.get_or("CC_MCP", "").trim().to_lowercase();
    if !matches!(mcp_val.as_str(), "off" | "none" | "0") {
        if let Some(reg) = mcp_registry_path() {
            cmd = format!("{cmd} --mcp-config {}", sh_quote(&reg.to_string_lossy()));
        }
    }
    if mcp_val == "chrome" {
        let chrome = home().join("mcp-chrome.json");
        if chrome.exists() {
            cmd = format!("{cmd} --mcp-config {}", sh_quote(&chrome.to_string_lossy()));
        }
    }
    if !cmd.contains("--model") {
        cmd = format!("{cmd} --model sonnet");
    }
    cmd
}

async fn start_session(state: &AppState, name: &str, extra_flags: &str, skip_conv_id: bool) -> (bool, String) {
    if !valid_session_name(name) {
        return (false, "invalid session name".into());
    }
    if is_session_blocked(name) {
        return (false, "session is blocked; remove it from blocked-sessions.txt first".into());
    }
    let f = env_path(name);
    if !f.exists() {
        return (false, format!("session '{name}' not found"));
    }
    let cfg = parse_env(name);
    if !iterm2_id(&cfg).is_empty() {
        return (false, "iTerm2-backed sessions are not supported by the rust origin yet".into());
    }
    if backend_of_cfg(&cfg) == "herdr" {
        return (
            false,
            "herdr-backed session start is not ported to the rust origin yet (gap named in api/session_verbs.rs)".into(),
        );
    }
    if is_running(name).await {
        return (true, "already running".into());
    }
    if cfg.get("CC_ARCHIVED") == Some("1") {
        return (false, "session is archived; wake it first".into());
    }
    let work_dir = {
        let wd = cfg.get_or("CC_DIR", "").trim();
        let wd = if wd.is_empty() {
            std::env::var("HOME").unwrap_or_default()
        } else {
            wd.to_string()
        };
        expanduser(&wd)
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| expanduser(&wd).to_string_lossy().into_owned())
    };
    let mut flags = cfg.get_or("CC_FLAGS", "").to_string();
    #[cfg(unix)]
    {
        // Claude Code rejects --dangerously-skip-permissions as root (py:24242).
        if libc_geteuid() == 0 && flags.contains("--dangerously-skip-permissions") {
            flags = flags.replace("--dangerously-skip-permissions", "").trim().to_string();
        }
    }
    let mut meta = load_meta(name);
    let provider = provider_of(&cfg);
    let uuid_re = regex::Regex::new(r"^[0-9a-fA-F-]{36}$").unwrap();
    // Resume strategy (py:24250-24295). The Rust origin resolves via the
    // conversation UUID (deterministic, hook-reported); the name-indexed
    // lookup Python layers on top needs its session-name index, which is a
    // coexistence gap — a stale UUID still falls back to a fresh --name start.
    let mut session_flag = String::new();
    if !skip_conv_id && provider == "claude" {
        let conv_id = meta_str(&meta, "cc_conversation_id");
        if !conv_id.is_empty() && uuid_re.is_match(&conv_id) {
            let conv_file =
                claude_home().join("projects").join(project_name(&work_dir)).join(format!("{conv_id}.jsonl"));
            if conv_file.exists() {
                session_flag = format!("--resume {conv_id}");
            }
        }
        if session_flag.is_empty() {
            session_flag = format!("--name {}", sh_quote(name));
        }
    }
    let defaults = EnvFile::load(&home().join("defaults.env"));
    let default_flags = defaults.get_or("CC_DEFAULT_FLAGS", "").to_string();

    let cmd = match provider.as_str() {
        "codex" => {
            // py:24380 — codex command construction (trust-db side effect not
            // ported).
            let codex_session_id = meta_str(&meta, "codex_session_id");
            let mut codex_flags = flags.clone();
            let codex_yolo = PROVIDER_YOLO_FLAGS.iter().any(|f| codex_flags.contains(f));
            if codex_yolo {
                codex_flags = strip_provider_yolo_flags(&codex_flags);
            }
            let mut opts = String::new();
            if !codex_flags.is_empty() {
                opts += &format!(" {}", shell_quote_flags(&codex_flags));
            }
            if !extra_flags.is_empty() {
                opts += &format!(" {}", shell_quote_flags(extra_flags));
            }
            if !opts.contains("--model") && !opts.contains("-m ") {
                opts += " --model gpt-5.5";
            }
            if !opts.contains("--dangerously-bypass") && !opts.contains("-a ") {
                opts += if codex_yolo { " --dangerously-bypass-approvals-and-sandbox" } else { " -a never" };
            }
            if !codex_yolo && !opts.contains("--dangerously-bypass") && !opts.contains("--sandbox") && !opts.contains("-s ") {
                opts += " --sandbox workspace-write";
            }
            let logs = logs_dir().to_string_lossy().into_owned();
            if !opts.contains(&logs) {
                opts += &format!(" --add-dir {}", sh_quote(&logs));
            }
            if let Some(gr) = run_cmd("git", &["-C", &work_dir, "rev-parse", "--show-toplevel"], OP_TIMEOUT).await {
                if gr.status.success() {
                    let root = String::from_utf8_lossy(&gr.stdout).trim().to_string();
                    if root != work_dir && !opts.contains(&root) {
                        opts += &format!(" --add-dir {}", sh_quote(&root));
                    }
                    let git_dir = format!("{root}/.git");
                    if Path::new(&git_dir).is_dir() && !opts.contains(&git_dir) {
                        opts += &format!(" --add-dir {}", sh_quote(&git_dir));
                    }
                }
            }
            if !codex_session_id.is_empty() {
                format!("codex resume{opts} {codex_session_id}")
            } else {
                format!("codex{opts}")
            }
        }
        "gemini" => {
            // py:24443 — gemini command (GEMINI.md memory bridge not ported).
            let mut gflags = flags.clone();
            let gyolo = PROVIDER_YOLO_FLAGS.iter().any(|f| gflags.contains(f))
                || gflags.contains("--approval-mode=yolo")
                || gflags.contains("--approval-mode yolo");
            if gyolo {
                gflags = strip_provider_yolo_flags(&gflags);
            }
            let mut opts = String::new();
            if !gflags.is_empty() {
                opts += &format!(" {}", shell_quote_flags(&gflags));
            }
            if !extra_flags.is_empty() {
                opts += &format!(" {}", shell_quote_flags(extra_flags));
            }
            if !opts.contains("--model") && !opts.contains("-m ") {
                opts += " --model auto";
            }
            if gyolo && !opts.contains("--yolo") && !opts.contains("--approval-mode") {
                opts += " --yolo";
            }
            if !opts.contains("--skip-trust") {
                opts += " --skip-trust";
            }
            let logs = logs_dir().to_string_lossy().into_owned();
            opts += &format!(" --include-directories {}", sh_quote(&logs));
            let gemini_session_id = meta_str(&meta, "gemini_session_id");
            if !gemini_session_id.is_empty() {
                format!("gemini{opts} --resume {}", sh_quote(&gemini_session_id))
            } else {
                let new_id = ulid::Ulid::new().to_string().to_lowercase();
                meta.insert("gemini_session_id".into(), json!(new_id));
                format!("gemini{opts} --session-id {}", sh_quote(&new_id))
            }
        }
        _ => build_claude_cmd(&cfg, &flags, &default_flags, &session_flag, extra_flags),
    };

    // Shell setup line (py:24532): unset Claude env markers, source profile,
    // cd, source the global agent credentials.
    let mut has_oauth = false;
    let mut shell_rc = String::new();
    if provider != "codex" && provider != "gemini" {
        shell_rc.push_str("unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT; ");
        if let Ok(t) = std::fs::read_to_string(PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude.json")) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                has_oauth = py_truthy(&v["oauthAccount"]);
            }
        }
        if has_oauth {
            shell_rc.push_str("unset ANTHROPIC_API_KEY; ");
        }
    }
    let home_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    for rc in [".zprofile", ".bash_profile", ".profile"] {
        let p = home_dir.join(rc);
        if p.exists() {
            shell_rc.push_str(&format!(
                "source {} 2>/dev/null; cd {}; ",
                sh_quote(&p.to_string_lossy()),
                sh_quote(&work_dir)
            ));
            break;
        }
    }
    let amux_env = home().join("amux.env");
    if amux_env.exists() {
        shell_rc.push_str(&format!(
            "set -a; source {} 2>/dev/null; set +a; ",
            sh_quote(&amux_env.to_string_lossy())
        ));
    } else {
        shell_rc.push_str(&format!("cd {}; ", sh_quote(&work_dir)));
    }
    if provider != "codex" && provider != "gemini" && has_oauth {
        shell_rc.push_str("unset ANTHROPIC_API_KEY; ");
    }
    let mut env_args: Vec<String> = Vec::new();
    if has_oauth {
        env_args.push("-e".into());
        env_args.push("ANTHROPIC_API_KEY=".into());
    } else if let Ok(v) = std::env::var("ANTHROPIC_API_KEY") {
        if !v.is_empty() {
            env_args.push("-e".into());
            env_args.push(format!("ANTHROPIC_API_KEY={v}"));
        }
    }
    for k in [
        "ANTHROPIC_API_BASE", "OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY",
        "GOOGLE_GENAI_USE_VERTEXAI", "GOOGLE_CLOUD_PROJECT", "GOOGLE_CLOUD_LOCATION",
    ] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                env_args.push("-e".into());
                env_args.push(format!("{k}={v}"));
            }
        }
    }

    let tmux_sess = tmux_name(name);
    let tmux_exists = tmux_sessions_set().await.contains(&tmux_sess);
    if tmux_exists {
        // Reuse the surviving tmux session (py:24589).
        let output = tmux_capture(name, 10).await;
        if at_shell_prompt(&strip_ansi(&output)) {
            send_key(name, "C-c").await;
            sleep_ms(100).await;
            send_key(name, "C-u").await;
            sleep_ms(100).await;
            type_line(name, "HISTFILE=/dev/null").await;
            poll_shell_prompt(name, 3000).await;
            type_line(name, &format!("cd {}", sh_quote(&work_dir))).await;
            poll_shell_prompt(name, 3000).await;
        } else {
            send_key(name, "C-c").await;
            sleep_ms(3000).await;
            let out2 = tmux_capture(name, 10).await;
            if !at_shell_prompt(&strip_ansi(&out2)) {
                let ptq = pt(name);
                let sh = user_shell();
                let _ = tmux(&["respawn-pane", "-k", "-t", &ptq, &sh]).await;
                sleep_ms(1000).await;
                type_line(name, &shell_rc).await;
                poll_shell_prompt(name, 3000).await;
            } else {
                send_key(name, "C-u").await;
                sleep_ms(100).await;
                type_line(name, "HISTFILE=/dev/null").await;
                poll_shell_prompt(name, 3000).await;
                type_line(name, &format!("cd {}", sh_quote(&work_dir))).await;
                poll_shell_prompt(name, 3000).await;
            }
        }
    } else {
        // Fresh tmux session hosting the user's login shell (py:24647).
        let cols = tmux_cols();
        let rows = tmux_rows();
        let scheme = if std::env::args().any(|a| a == "--no-tls") { "http" } else { "https" };
        let mut args: Vec<String> = vec![
            "new-session".into(), "-d".into(), "-s".into(), tmux_sess.clone(),
            "-n".into(), name.into(), "-c".into(), work_dir.clone(),
            "-x".into(), cols, "-y".into(), rows,
            "-e".into(), format!("TMUX_SESSION_NAME={name}"),
            "-e".into(), format!("AMUX_WORKER={name}"),
            "-e".into(), format!("AMUX_SESSION={name}"),
            "-e".into(), format!("AMUX_URL={scheme}://localhost:8822"),
        ];
        args.extend(env_args.iter().cloned());
        args.push(user_shell());
        let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
        match run_cmd("tmux", &args_ref, Duration::from_secs(10)).await {
            Some(o) if o.status.success() => {}
            Some(o) => return (false, String::from_utf8_lossy(&o.stderr).into_owned()),
            None => return (false, "tmux not found or timed out".into()),
        }
        let stq = st(name);
        let _ = tmux(&["set-option", "-t", &stq, "remain-on-exit", "on"]).await;
        let _ = tmux(&["set-option", "-t", &stq, "allow-rename", "off"]).await;
        let _ = tmux(&["set-window-option", "-t", &stq, "automatic-rename", "off"]).await;
        let _ = tmux(&["rename-window", "-t", &stq, name]).await;
        type_line(name, &shell_rc).await;
        poll_shell_prompt(name, 3000).await;
    }
    if has_oauth && provider != "codex" && provider != "gemini" {
        type_line(name, "unset ANTHROPIC_API_KEY").await;
        poll_shell_prompt(name, 3000).await;
    }
    // Launch the provider command.
    let _ = send_literal(name, &cmd).await;
    sleep_ms(150).await;
    send_key(name, "Enter").await;
    // Wait for the agent UI (py:24717).
    let mut launched = false;
    for i in 0..20 {
        sleep_ms(500).await;
        let out = tmux_capture(name, 10).await;
        if !out.is_empty() {
            let clean = strip_ansi(&out);
            if claude_ui_visible(&clean) {
                launched = true;
                break;
            }
            if i >= 10 && at_shell_prompt(&clean) {
                break;
            }
            if i >= 6 && at_resume_picker(&clean) {
                break;
            }
        }
    }
    if !launched && !skip_conv_id && provider == "claude" {
        let mut out_check = strip_ansi(&tmux_capture(name, 10).await);
        if at_resume_picker(&out_check) {
            // Escape the picker, drop the stale ids, fall through to fresh.
            send_key(name, "Escape").await;
            sleep_ms(500).await;
            send_key(name, "C-c").await;
            sleep_ms(2000).await;
            for _ in 0..10 {
                let o = strip_ansi(&tmux_capture(name, 10).await);
                if at_shell_prompt(&o) {
                    break;
                }
                sleep_ms(500).await;
            }
            meta.remove("cc_session_name");
            meta.remove("cc_conversation_id");
            save_meta(name, &meta);
            out_check = strip_ansi(&tmux_capture(name, 10).await);
        }
        if at_shell_prompt(&out_check) {
            // --resume failed: fresh start with --name (py:24762).
            meta.remove("cc_session_name");
            meta.remove("cc_conversation_id");
            save_meta(name, &meta);
            send_key(name, "C-c").await;
            sleep_ms(100).await;
            send_key(name, "C-u").await;
            sleep_ms(100).await;
            let fresh_flag = format!("--name {}", sh_quote(name));
            let cmd_fresh = build_claude_cmd(&cfg, &flags, &default_flags, &fresh_flag, extra_flags);
            let _ = send_literal(name, &cmd_fresh).await;
            sleep_ms(150).await;
            send_key(name, "Enter").await;
            for _ in 0..10 {
                sleep_ms(500).await;
                let out2 = tmux_capture(name, 10).await;
                if !out2.is_empty() && claude_ui_visible(&strip_ansi(&out2)) {
                    launched = true;
                    break;
                }
            }
            if !launched {
                let out3 = strip_ansi(&tmux_capture(name, 10).await);
                if at_shell_prompt(&out3) {
                    meta.insert("start_error".into(), json!("both resume and fresh start failed"));
                    save_meta(name, &meta);
                    return (false, "Claude failed to start".into());
                }
            }
        }
    }
    // Stream output to the session log (py:24800).
    let _ = std::fs::create_dir_all(logs_dir());
    let lp = log_path(name);
    {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&lp) {
            let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
            let _ = f.write_all(format!("\n\n=== Session started: {ts} ===\n\n").as_bytes());
        }
    }
    let ptq = pt(name);
    let pipe_cmd = log_pipe_command(&lp);
    let _ = tmux(&["pipe-pane", "-t", &ptq, "-o", &pipe_cmd]).await;
    meta.remove("start_error");
    meta.insert("last_started".into(), json!(now_i64()));
    let count = meta.get("start_count").and_then(|v| v.as_i64()).unwrap_or(0);
    meta.insert("start_count".into(), json!(count + 1));
    let pending_reload = meta.remove("pending_log_reload").is_some();
    let pending_reason = meta
        .remove("pending_log_reload_reason")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    save_meta(name, &meta);
    if pending_reload && log_path(name).exists() {
        let prompt = log_reload_prompt(name, &pending_reason);
        let st2 = state.clone();
        let n = name.to_string();
        tokio::spawn(async move { send_after_ready(st2, n, prompt, 60).await });
    }
    // Standing instruction re-send (py:24833). Board digest briefing: gap.
    let instr = meta_str(&load_meta(name), "instructions").trim().to_string();
    if !instr.is_empty() {
        let st2 = state.clone();
        let n = name.to_string();
        tokio::spawn(async move { send_after_ready(st2, n, instr, 60).await });
    }
    emit_event(
        state,
        name,
        "session.started",
        Some(json!({"resumed": !meta_str(&load_meta(name), "cc_conversation_id").is_empty()})),
        None,
        "start_session",
    )
    .await;
    (true, "started".into())
}

#[cfg(unix)]
fn libc_geteuid() -> u32 {
    // std has no geteuid without the libc crate; the UID check only matters
    // for root containers. Read /proc-less macOS via `id -u` once.
    use std::sync::OnceLock;
    static UID: OnceLock<u32> = OnceLock::new();
    *UID.get_or_init(|| {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(1000)
    })
}

fn log_reload_prompt(name: &str, reason: &str) -> String {
    let lp = log_path(name);
    let size = lp.metadata().map(|m| m.len()).unwrap_or(0);
    let size_mb = size as f64 / (1024.0 * 1024.0);
    let cap_mb = MAX_LOG_BYTES / (1024 * 1024);
    let reason_text = if reason.is_empty() { "session swap" } else { reason };
    format!(
        "Before continuing, load the previous amux terminal context.\n\n\
         The log tail captured for this {reason_text} is at:\n{}\n\n\
         Read that file now. It contains up to the last {cap_mb} MB of this \
         session's terminal history ({size_mb:.1} MB currently saved). Use it \
         as continuity context for the work in this session. Do not summarize it \
         back unless asked.",
        lp.display()
    )
}

// ---------------------------------------------------------------------------
// stop_session (py:24943): record the resumable name, /exit gracefully, wait
// for the shell, hard-kill on timeout. tmux stays alive.
// ---------------------------------------------------------------------------

async fn stop_session(name: &str) -> (bool, String) {
    if !valid_session_name(name) {
        return (false, "invalid session name".into());
    }
    let cfg = parse_env(name);
    if backend_of_cfg(&cfg) == "herdr" {
        if !herdr_agent_running(name).await {
            return (true, "not running".into());
        }
        let an = herdr_agent_name(name);
        let mut meta = load_meta(name);
        if meta_str(&meta, "cc_session_name").is_empty() {
            meta.insert("cc_session_name".into(), json!(name));
            save_meta(name, &meta);
        }
        let _ = herdr_json(&["agent", "send-keys", &an, "ctrl+u"], OP_TIMEOUT).await;
        sleep_ms(100).await;
        let _ = herdr_json(&["agent", "prompt", &an, "/exit"], Duration::from_secs(10)).await;
        for _ in 0..30 {
            sleep_ms(500).await;
            if !herdr_agent_running(name).await {
                return (true, "stopped".into());
            }
        }
        return (true, "stopped (hard-kill unavailable on rust origin — pane close is a gap)".into());
    }
    let tmux_sess = tmux_name(name);
    if !tmux_sessions_set().await.contains(&tmux_sess) {
        return (true, "not running".into());
    }
    let output = tmux_capture(name, 10).await;
    let mut meta = load_meta(name);
    if at_shell_prompt(&strip_ansi(&output)) {
        if meta_str(&meta, "cc_session_name").is_empty() {
            meta.insert("cc_session_name".into(), json!(name));
            save_meta(name, &meta);
        }
        return (true, "not running".into());
    }
    // Claude-pid/session-name introspection (py:24974 reads the running
    // process's name file) is not ported; the /rename fallback below pins the
    // resumable name to the amux name, which is what the fresh-start path
    // records anyway.
    let _ = send_literal(name, &format!("/rename {name}")).await;
    sleep_ms(150).await;
    send_key(name, "Enter").await;
    sleep_ms(800).await;
    meta.insert("cc_session_name".into(), json!(name));
    save_meta(name, &meta);
    // Detach pipe-pane before shell-visible commands (py:24995).
    let stq = st(name);
    let _ = tmux(&["pipe-pane", "-t", &stq]).await;
    send_key(name, "C-u").await;
    sleep_ms(100).await;
    let _ = send_literal(name, "/exit").await;
    sleep_ms(150).await;
    send_key(name, "Enter").await;
    for _ in 0..30 {
        sleep_ms(500).await;
        let out = tmux_capture(name, 10).await;
        if at_shell_prompt(&strip_ansi(&out)) {
            return (true, "stopped".into());
        }
    }
    // Hard kill: the pane shell's children (py:25028).
    if let Some(out) = tmux(&["list-panes", "-t", &stq, "-F", "#{pane_pid}"]).await {
        if out.status.success() {
            let pid = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
            if !pid.is_empty() {
                let _ = run_cmd("pkill", &["-9", "-P", &pid], OP_TIMEOUT).await;
            }
        }
    }
    type_line(name, "stty sane").await;
    sleep_ms(1000).await;
    (true, "stopped (hard-kill)".into())
}

async fn kill_tmux_session(name: &str) {
    let stq = st(name);
    let _ = tmux(&["kill-session", "-t", &stq]).await;
}

/// py:25055 archive_session — scrollback→log, stop, kill tmux, CC_ARCHIVED=1,
/// card cascade.
async fn archive_session(state: &AppState, name: &str) -> (bool, String) {
    let f = env_path(name);
    if !f.exists() {
        return (false, format!("session '{name}' not found"));
    }
    let cfg = parse_env(name);
    if is_running(name).await {
        let raw = if backend_of_cfg(&cfg) == "herdr" {
            herdr_capture(name, 50000).await
        } else {
            let ptq = pt(name);
            match run_cmd("tmux", &["capture-pane", "-t", &ptq, "-p", "-S", "-"], Duration::from_secs(30)).await {
                Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
                _ => String::new(),
            }
        };
        if !raw.trim().is_empty() {
            let _ = std::fs::create_dir_all(logs_dir());
            let data = raw.into_bytes();
            let start = data.len().saturating_sub(MAX_LOG_BYTES);
            let _ = std::fs::write(log_path(name), &data[start..]);
        }
        let _ = stop_session(name).await;
    }
    kill_tmux_session(name).await;
    let mut cfg = parse_env(name);
    cfg.set("CC_ARCHIVED", "1");
    if cfg.write(&f).is_err() {
        return (false, "could not write session env".into());
    }
    archive_session_issues(state, name, 1).await;
    (true, "archived".into())
}

/// py:25107 _archive_session_issues — flip the archived bit on the lane's
/// cards, both directions.
async fn archive_session_issues(state: &AppState, name: &str, flag: i64) {
    let name = name.to_string();
    let _ = state
        .store
        .write_async(move |conn| {
            let n = conn
                .execute(
                    "UPDATE issues SET archived=?1, updated=?2 WHERE session=?3 AND deleted IS NULL AND archived!=?1",
                    rusqlite::params![flag, now_i64(), name],
                )
                .unwrap_or(0);
            Ok(crate::db::WriteOutcome {
                applied: n > 0,
                events: if n > 0 {
                    vec![crate::db::PendingEvent {
                        entity_type: amux_core::revision::EntityType::Other("issue".into()),
                        entity_id: name.clone(),
                        mutation: amux_core::revision::MutationKind::Updated,
                        payload: None,
                    }]
                } else {
                    vec![]
                },
            })
        })
        .await;
}

/// py:25137 reset_session — drop the conversation, keep the lane.
async fn reset_session(state: &AppState, name: &str) -> (bool, String) {
    let f = env_path(name);
    if !f.exists() {
        return (false, format!("session '{name}' not found"));
    }
    if is_session_blocked(name) {
        return (false, "session is blocked; remove it from blocked-sessions.txt first".into());
    }
    if is_running(name).await {
        let ptq = pt(name);
        if let Some(o) = run_cmd("tmux", &["capture-pane", "-t", &ptq, "-p", "-S", "-"], Duration::from_secs(30)).await {
            let raw = String::from_utf8_lossy(&o.stdout).into_owned();
            if !raw.trim().is_empty() {
                let data = raw.into_bytes();
                let start = data.len().saturating_sub(MAX_LOG_BYTES);
                let _ = std::fs::write(log_path(name), &data[start..]);
            }
        }
        let _ = stop_session(name).await;
        kill_tmux_session(name).await;
    }
    let mut meta = load_meta(name);
    meta.remove("cc_conversation_id");
    meta.remove("cc_session_name");
    save_meta(name, &meta);
    let (ok, msg) = start_session(state, name, "", false).await;
    if ok {
        (true, "reset — fresh conversation, lane intact".into())
    } else {
        (false, msg)
    }
}

/// py:25184 wake_session — clear CC_ARCHIVED, un-archive cards, start.
async fn wake_session(state: &AppState, name: &str) -> (bool, String) {
    if is_session_blocked(name) {
        return (false, "session is blocked; remove it from blocked-sessions.txt first".into());
    }
    let f = env_path(name);
    if !f.exists() {
        return (false, format!("session '{name}' not found"));
    }
    let mut cfg = parse_env(name);
    cfg.remove("CC_ARCHIVED");
    if cfg.write(&f).is_err() {
        return (false, "could not write session env".into());
    }
    archive_session_issues(state, name, 0).await;
    start_session(state, name, "", false).await
}

/// py:25832 _resize_pane — refuse when a real client is attached.
async fn resize_pane(name: &str, cols: i64, rows: i64) -> (bool, String) {
    let cols = cols.clamp(50, 300);
    let rows = rows.clamp(20, 100);
    let stq = st(name);
    if let Some(o) = tmux(&["list-clients", "-t", &stq, "-F", "#{client_name}"]).await {
        if o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty() {
            return (false, "terminal client attached — its size wins".into());
        }
    }
    if let Some(o) = tmux(&["display-message", "-p", "-t", &stq, "#{window_width}x#{window_height}"]).await {
        if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == format!("{cols}x{rows}") {
            return (true, "already sized".into());
        }
    }
    let cs = cols.to_string();
    let rs = rows.to_string();
    let _ = tmux(&["resize-window", "-t", &stq, "-x", &cs, "-y", &rs]).await;
    (true, format!("resized to {cols}x{rows}"))
}

// ---------------------------------------------------------------------------
// Agent panel navigation (py:25861-25959) — every key gated on a fresh
// capture; the Background dialog is cancelled, never confirmed.
// ---------------------------------------------------------------------------

struct AgentRow {
    cursor: bool,
    viewed: bool,
    label: String,
}

fn agent_panel(clean: &str) -> (bool, Vec<AgentRow>) {
    let lines: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut rows: Vec<AgentRow> = Vec::new();
    let gap_re = regex::Regex::new(r"\s{4,}").unwrap();
    let ws_re = regex::Regex::new(r"\s+").unwrap();
    let mut i = lines.len() as isize - 1;
    while i >= 0 {
        let s = lines[i as usize].trim();
        let body = s.trim_start_matches(['\u{276f}', ' ', '\u{a0}']).trim();
        let first = body.chars().next();
        if matches!(first, Some('\u{23fa}' | '\u{25ef}' | '\u{25cf}' | '\u{25cb}')) {
            let after: String = body.chars().skip(1).collect();
            let label = gap_re.split(after.trim()).next().unwrap_or("").to_string();
            rows.push(AgentRow {
                cursor: s.starts_with('\u{276f}'),
                viewed: matches!(first, Some('\u{23fa}' | '\u{25cf}')),
                label: ws_re.replace_all(&label, " ").into_owned(),
            });
            i -= 1;
            continue;
        }
        break;
    }
    rows.reverse();
    let select = rows.iter().any(|r| r.cursor);
    (select, rows)
}

async fn agent_nav(name: &str, direction: &str, index: i64) -> (bool, String) {
    let cap = |name: String| async move { strip_ansi(&tmux_capture(&name, 20).await) };
    let mut clean = cap(name.to_string()).await;
    if clean.contains("Background this session?") {
        send_key(name, "Escape").await;
        sleep_ms(400).await;
        clean = cap(name.to_string()).await;
    }
    let (mut select, mut rows) = agent_panel(&clean);
    if rows.is_empty() {
        return (false, "no agents panel on screen".into());
    }
    for _ in 0..3 {
        if select {
            break;
        }
        send_key(name, "Down").await;
        sleep_ms(350).await;
        let c = cap(name.to_string()).await;
        let (s2, r2) = agent_panel(&c);
        select = s2;
        rows = r2;
    }
    if !select || rows.is_empty() {
        return (false, "could not enter agent select mode".into());
    }
    let cursor = rows
        .iter()
        .position(|r| r.cursor)
        .or_else(|| rows.iter().position(|r| r.viewed))
        .unwrap_or(0);
    let target = match direction {
        "main" => 0usize,
        "index" => (index.max(0) as usize).min(rows.len() - 1),
        "up" => cursor.saturating_sub(1),
        _ => (cursor + 1).min(rows.len() - 1),
    };
    let steps = target.abs_diff(cursor);
    for _ in 0..steps {
        send_key(name, if target > cursor { "Down" } else { "Up" }).await;
        sleep_ms(250).await;
    }
    let c = cap(name.to_string()).await;
    let (s3, _r3) = agent_panel(&c);
    if !s3 {
        return (false, "agent panel closed mid-navigation".into());
    }
    send_key(name, "Enter").await;
    sleep_ms(350).await;
    let c2 = cap(name.to_string()).await;
    let (_s4, rows4) = agent_panel(&c2);
    let viewing = rows4.iter().find(|r| r.viewed).map(|r| r.label.clone()).unwrap_or_default();
    (true, if viewing.is_empty() { "switched".into() } else { viewing })
}

// ---------------------------------------------------------------------------
// Saved-log helpers (py:5175-5478).
// ---------------------------------------------------------------------------

fn load_session_log(name: &str, tail_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let lp = log_path(name);
    let Ok(mut f) = std::fs::File::open(&lp) else { return String::new() };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if tail_bytes > 0 && size > tail_bytes && f.seek(SeekFrom::Start(size - tail_bytes)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// py:5175 save_session_log (throttle omitted — the rust origin saves on the
/// peek path only when it actually captured something new; the Python
/// throttle exists to protect a 10MB rewrite loop under polling, which the
/// append path below avoids equally well by being append-only until cap).
fn save_session_log(name: &str, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(logs_dir());
    let lp = log_path(name);
    let data = content.as_bytes();
    let existing = lp.metadata().map(|m| m.len() as usize).unwrap_or(0);
    if existing + data.len() > MAX_LOG_BYTES {
        let mut combined = std::fs::read(&lp).unwrap_or_default();
        combined.extend_from_slice(data);
        let start = combined.len().saturating_sub(MAX_LOG_BYTES);
        let _ = std::fs::write(&lp, &combined[start..]);
    } else {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&lp) {
            let _ = f.write_all(data);
        }
    }
}

/// py:5464 _write_plain_log — ANSI-stripped mirror for the session to Read.
fn write_plain_log(name: &str) -> Option<(PathBuf, usize)> {
    let lp = log_path(name);
    if !lp.exists() {
        return None;
    }
    let text = std::fs::read_to_string(&lp)
        .unwrap_or_else(|_| String::from_utf8_lossy(&std::fs::read(&lp).unwrap_or_default()).into_owned());
    let clean = collapse_blank_runs(&strip_ansi(&text));
    let cp = plain_log_path(name);
    std::fs::create_dir_all(cp.parent()?).ok()?;
    std::fs::write(&cp, clean.as_bytes()).ok()?;
    Some((cp, clean.len()))
}

/// py:22616 _capture_log_tail_for_reload — persist the last MAX_LOG_BYTES of
/// output before a provider/model/effort/yolo swap.
async fn capture_log_tail_for_reload(name: &str, reason: &str) -> bool {
    if !valid_session_name(name) {
        return false;
    }
    let _ = std::fs::create_dir_all(logs_dir());
    let lp = log_path(name);
    let mut chunks: Vec<u8> = Vec::new();
    let existing = load_session_log(name, MAX_LOG_BYTES as u64);
    chunks.extend_from_slice(existing.as_bytes());
    let mut captured = String::new();
    if is_running(name).await {
        let stq = st(name);
        let _ = tmux(&["pipe-pane", "-t", &stq]).await;
        let ptq = pt(name);
        if let Some(o) = run_cmd("tmux", &["capture-pane", "-t", &ptq, "-p", "-S", "-"], Duration::from_secs(30)).await {
            captured = String::from_utf8_lossy(&o.stdout).into_owned();
        }
    }
    if !captured.trim().is_empty() {
        let safe_reason = reason.replace('\n', " ").trim().to_string();
        let safe_reason = if safe_reason.is_empty() { "session swap".to_string() } else { safe_reason };
        let ts = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
        let marker = format!("\n\n=== Captured before {safe_reason}: {ts} ===\n\n");
        let cap_text = if tmux_alt_screen(name).await {
            collapse_blank_runs(&captured)
        } else {
            captured
        };
        chunks.extend_from_slice(marker.as_bytes());
        chunks.extend_from_slice(cap_text.as_bytes());
    }
    if chunks.is_empty() {
        return false;
    }
    let start = chunks.len().saturating_sub(MAX_LOG_BYTES);
    std::fs::write(&lp, &chunks[start..]).is_ok()
}

fn mark_pending_log_reload(name: &str, reason: &str) {
    update_meta(
        name,
        &[("pending_log_reload", json!(now_i64())), ("pending_log_reload_reason", json!(reason))],
    );
}

// ---------------------------------------------------------------------------
// get_claude_stats (py:9619), cc tasks (py:5505), git info (py:21236).
// ---------------------------------------------------------------------------

fn get_claude_stats(work_dir: &str) -> Value {
    if work_dir.is_empty() {
        return json!({"tokens": 0, "last_active": ""});
    }
    let project_dir = claude_home().join("projects").join(project_name(work_dir));
    let Ok(rd) = std::fs::read_dir(&project_dir) else {
        return json!({"tokens": 0, "last_active": ""});
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((_, newest)) = files.first() else {
        return json!({"tokens": 0, "last_active": ""});
    };
    let mut total_in: i64 = 0;
    let mut total_out: i64 = 0;
    let mut last_ts = String::new();
    for entry in iter_jsonl_tail(newest, 5_000_000) {
        if let Some(ts) = entry["timestamp"].as_str() {
            if !ts.is_empty() {
                last_ts = ts.to_string();
            }
        }
        let usage = &entry["message"]["usage"];
        if usage.is_object() {
            total_in += usage["input_tokens"].as_i64().unwrap_or(0);
            total_in += usage["cache_read_input_tokens"].as_i64().unwrap_or(0);
            total_out += usage["output_tokens"].as_i64().unwrap_or(0);
        }
    }
    json!({"tokens": total_in + total_out, "last_active": last_ts})
}

fn plan_stale_hide_secs() -> f64 {
    std::env::var("AMUX_PLAN_STALE_HIDE_HOURS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(24.0) * 3600.0
}

/// py:5505 _session_cc_tasks — Claude Code's native task list, read-only.
async fn session_cc_tasks(name: &str) -> Value {
    let empty = json!({"tasks": [], "counts": {}, "active": Value::Null, "total": 0});
    let owner = meta_str(&load_meta(name), "cc_session_name");
    if !owner.is_empty() && owner != name {
        return json!({"tasks": [], "counts": {}, "active": Value::Null, "total": 0,
                      "_suppressed": format!("cross-linked to {owner}")});
    }
    let Some(p) = session_jsonl_path(name) else { return empty };
    let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { return empty };
    let tdir = claude_home().join("tasks").join(stem);
    if !tdir.is_dir() {
        return empty;
    }
    // Fresh-splash guard (py:5540): a brand-new conversation with no turns
    // must not surface the dead conversation's plan.
    let raw = tmux_capture(name, 40).await;
    if !raw.is_empty() {
        let clean = strip_ansi(&raw);
        if let Some(i) = clean.rfind("Claude Code v") {
            let after = &clean[i..];
            if !after.contains('\u{23fa}') && !after.contains('\u{25cf}') {
                return empty;
            }
        }
    }
    let mut tasks: Vec<(f64, Value)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&tdir) {
        for e in rd.flatten() {
            let path = e.path();
            let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !fname.ends_with(".json") || !fname.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let Ok(d) = serde_json::from_str::<Value>(&text) else { continue };
            if !d.is_object() {
                continue;
            }
            let mtime = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let stem = fname.trim_end_matches(".json");
            let id = d["id"].as_str().map(String::from).unwrap_or_else(|| {
                if d["id"].is_number() { d["id"].to_string() } else { stem.to_string() }
            });
            tasks.push((
                mtime,
                json!({
                    "id": id,
                    "subject": d["subject"].as_str().or(d["activeForm"].as_str()).unwrap_or("").trim(),
                    "activeForm": d["activeForm"].as_str().unwrap_or("").trim(),
                    "status": if d["status"].is_string() { d["status"].clone() } else { json!("pending") },
                    "blockedBy": d["blockedBy"].as_array().map(|a| a.iter().map(|x| json!(x.as_str().map(String::from).unwrap_or_else(|| x.to_string()))).collect::<Vec<_>>()).unwrap_or_default(),
                }),
            ));
        }
    }
    tasks.sort_by_key(|(_, t)| {
        t["id"].as_str().and_then(|s| s.parse::<i64>().ok()).unwrap_or(1_000_000)
    });
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for (_, t) in &tasks {
        *counts.entry(t["status"].as_str().unwrap_or("pending").to_string()).or_insert(0) += 1;
    }
    let active = tasks
        .iter()
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, t)| t.clone())
        .unwrap_or(Value::Null);
    let updated_at = tasks.iter().map(|(m, _)| *m).fold(0.0_f64, f64::max) as i64;
    if updated_at > 0 && now_f64() - updated_at as f64 > plan_stale_hide_secs() {
        return empty;
    }
    json!({
        "tasks": tasks.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(),
        "counts": counts,
        "active": active,
        "total": tasks.len(),
        "updated_at": updated_at,
    })
}

async fn git_out(wd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut full = vec!["-C", wd];
    full.extend_from_slice(args);
    let out = run_cmd("git", &full, timeout).await?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        None
    }
}

/// py:21236 _git_info (cache omitted — one caller per request here; the
/// Python cache defends a 60-session polling loop this origin doesn't run).
async fn git_info(work_dir: &str, detail: bool) -> Value {
    if work_dir.is_empty() {
        return json!({"branch": "", "repo": ""});
    }
    let branch = git_out(work_dir, &["branch", "--show-current"], Duration::from_secs(2))
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let repo = if branch.is_empty() {
        String::new()
    } else {
        git_out(work_dir, &["rev-parse", "--show-toplevel"], Duration::from_secs(2))
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut result = json!({"branch": branch, "repo": repo});
    if !detail || branch.is_empty() {
        return result;
    }
    let mut ahead_base = String::new();
    let mut ahead: Vec<String> = Vec::new();
    for base in ["main", "master", "dev", "develop"] {
        if let Some(out) = git_out(work_dir, &["log", &format!("{base}..HEAD"), "--oneline", "--no-decorate"], OP_TIMEOUT).await {
            ahead_base = base.to_string();
            ahead = out.trim().lines().filter(|l| !l.is_empty()).map(String::from).collect();
            break;
        }
    }
    result["ahead_base"] = json!(ahead_base);
    result["ahead"] = json!(ahead);
    let status: Vec<String> = git_out(work_dir, &["status", "--short"], OP_TIMEOUT)
        .await
        .map(|o| o.trim().lines().filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    result["dirty"] = json!(!status.is_empty());
    result["status"] = json!(status);
    fn parse_numstat(out: &str) -> Vec<Value> {
        out.trim()
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if parts.len() == 3 {
                    Some(json!({
                        "file": parts[2],
                        "added": parts[0].parse::<i64>().unwrap_or(0),
                        "deleted": parts[1].parse::<i64>().unwrap_or(0),
                    }))
                } else {
                    None
                }
            })
            .collect()
    }
    result["files_unstaged"] = json!(
        git_out(work_dir, &["diff", "--numstat"], OP_TIMEOUT).await.map(|o| parse_numstat(&o)).unwrap_or_default()
    );
    result["files_staged"] = json!(
        git_out(work_dir, &["diff", "--cached", "--numstat"], OP_TIMEOUT)
            .await
            .map(|o| parse_numstat(&o))
            .unwrap_or_default()
    );
    if !ahead_base.is_empty() && !ahead.is_empty() {
        result["files_committed"] = json!(
            git_out(work_dir, &["diff", &format!("{ahead_base}..HEAD"), "--numstat"], OP_TIMEOUT)
                .await
                .map(|o| parse_numstat(&o))
                .unwrap_or_default()
        );
    } else {
        result["files_committed"] = json!([]);
    }
    result["remote_url"] = json!(
        git_out(work_dir, &["remote", "get-url", "origin"], Duration::from_secs(3))
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    );
    result["unpushed"] = json!(
        git_out(work_dir, &["log", "@{u}..HEAD", "--oneline", "--no-decorate"], OP_TIMEOUT)
            .await
            .map(|o| o.trim().lines().filter(|l| !l.is_empty()).count())
            .unwrap_or(0)
    );
    result
}

/// py:18858/18877 — dirty files scoped to this session's territory.
fn all_session_workdirs() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("env") {
                continue;
            }
            let Some(n) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            let wd = session_work_dir(n);
            if !wd.is_empty() {
                out.insert(n.to_string(), wd);
            }
        }
    }
    out
}

async fn session_dirty_files(name: &str, work_dir: &str) -> Vec<String> {
    let wd = expanduser(work_dir)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| work_dir.to_string());
    let mut args: Vec<String> = vec!["status".into(), "--porcelain".into(), "--".into(), ".".into()];
    for (other, od) in all_session_workdirs() {
        if other == name {
            continue;
        }
        if od != wd && format!("{od}/").starts_with(&format!("{wd}/")) {
            if let Ok(rel) = Path::new(&od).strip_prefix(&wd) {
                args.push(format!(":(exclude){}", rel.display()));
            }
        }
    }
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    match git_out(&wd, &args_ref, Duration::from_secs(10)).await {
        Some(out) => out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.chars().skip(3).collect::<String>().trim().to_string())
            .collect(),
        None => vec![],
    }
}

// ---------------------------------------------------------------------------
// peek (py:74985) — the full response assembly. Server-side caches are
// omitted (Python's defend a polling dashboard against re-rendering a 120KB
// transcript; the render below is bounded and this origin serves one client
// per request — reintroduce a cache if the SPA's poll cadence lands here).
// ---------------------------------------------------------------------------

async fn peek_response(name: &str, lines: i64, live_only: bool, no_trim: bool) -> Value {
    let provider = provider_of(&parse_env(name));
    if live_only {
        let output = strip_scroll_pill(&tmux_capture(name, lines).await);
        let live = if output.is_empty() { String::new() } else { strip_launch_noise(output.trim()) };
        // The live=1 trim needs the transcript the CLIENT is displaying; the
        // rust origin re-renders it (bounded) instead of a process cache.
        let live = if !live.is_empty() && !no_trim {
            let tr = render_session_transcript(name, 120_000);
            if tr.is_empty() { live } else { trim_live_overlap(&tr, &live) }
        } else {
            live
        };
        let lv = if live.is_empty() { "(no output)".to_string() } else { collapse_blank_runs(&live) };
        return json!({"name": name, "live_only": true, "live": lv, "output": lv});
    }
    let mut output = strip_scroll_pill(&tmux_capture(name, lines).await);
    if provider == "gemini" {
        output = clean_gemini_frame(&tmux_capture(name, 0).await);
    }
    let tmux_lines = if output.is_empty() { 0 } else { output.lines().count() };
    let is_alt = tmux_alt_screen(name).await;
    if is_alt {
        let (transcript, output) = if provider != "claude" {
            // Non-Claude alt-screen TUIs repaint in place: the LIVE frame is
            // the whole truthful state (py:75040).
            (String::new(), clean_gemini_frame(&tmux_capture(name, 0).await))
        } else {
            (render_session_transcript(name, 120_000), output)
        };
        let mut live = if output.is_empty() { String::new() } else { strip_launch_noise(output.trim()) };
        if !transcript.is_empty() && !live.is_empty() {
            live = trim_live_overlap(&transcript, &live);
        }
        let live_out = if !live.is_empty() {
            collapse_blank_runs(&live)
        } else if transcript.is_empty() {
            "(no output)".to_string()
        } else {
            String::new()
        };
        // AMUX-1807: `output` mirrors the CURRENT terminal frame for API
        // consumers; never empty for a running session.
        let mut out_compat = live_out.clone();
        if out_compat.is_empty() && !output.is_empty() {
            out_compat = collapse_blank_runs(&strip_launch_noise(output.trim()));
        }
        let history = if transcript.is_empty() { String::new() } else { collapse_blank_runs(&transcript) };
        let ol = out_compat.lines().filter(|l| !l.trim().is_empty()).count();
        let hl = history.lines().filter(|l| !l.trim().is_empty()).count();
        let mut resp = json!({
            "name": name,
            "history": history,
            "live": live_out,
            "output": out_compat,
            "output_lines": ol,
            "history_lines": hl,
            // `output` is the CURRENT TERMINAL FRAME — never scrollback. A
            // full-screen prompt clears the screen and `output` collapses to
            // the modal (the 2026-07-27 "swallowed message" diagnosis). State
            // the structural fact instead of guessing at the cause.
            "output_is_viewport_only": true,
        });
        if hl > ol + 20 {
            resp["hint"] = json!(format!(
                "`output` is only the current terminal frame ({ol} line(s)) — a full-screen \
                 prompt can push all of a session's work off-viewport. Read `history` \
                 ({hl} lines) for what it was actually doing."
            ));
        }
        return resp;
    }
    // Normal screen (py:75117).
    if !output.is_empty() && tmux_lines >= 30 {
        save_session_log(name, &output);
        return json!({"name": name, "output": collapse_blank_runs(&strip_launch_noise(&output))});
    }
    let mut saved = load_session_log(name, 65_536);
    if !saved.is_empty() && log_looks_torn(&saved) {
        let clean = render_session_transcript(name, 120_000);
        if !clean.is_empty() {
            saved = clean;
        }
    }
    if !saved.is_empty() {
        let live = if output.is_empty() { String::new() } else { strip_launch_noise(output.trim()) };
        let combined = if !live.is_empty() && !saved.trim_end().ends_with(&live) {
            format!("{}\n\n{}\n", saved.trim_end(), live)
        } else {
            saved
        };
        return json!({"name": name, "output": collapse_blank_runs(&combined), "saved": true});
    }
    let fallback = if output.is_empty() { "(no output)".to_string() } else { output };
    json!({"name": name, "output": collapse_blank_runs(&fallback)})
}

// ---------------------------------------------------------------------------
// Misc verb support: transcripts backup (py:6112), memory sharing (py:21012),
// inherited instruction files (py:21782).
// ---------------------------------------------------------------------------

fn backup_session_jsonl(name: &str, reason: &str) -> Option<String> {
    let wd = session_work_dir(name);
    if wd.is_empty() {
        return None;
    }
    let project_dir = claude_home().join("projects").join(project_name(&wd));
    let Ok(rd) = std::fs::read_dir(&project_dir) else { return None };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let (_, src) = files.first()?;
    let dest_dir = transcripts_dir().join(name);
    std::fs::create_dir_all(&dest_dir).ok()?;
    let ts = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let dest = dest_dir.join(format!("{ts}_{reason}_{}", src.file_name()?.to_string_lossy()));
    std::fs::copy(src, &dest).ok()?;
    // Prune to the newest 20 (py:6142).
    if let Ok(rd) = std::fs::read_dir(&dest_dir) {
        let mut backups: Vec<(std::time::SystemTime, PathBuf)> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
            .collect();
        backups.sort_by(|a, b| a.0.cmp(&b.0));
        let excess = backups.len().saturating_sub(20);
        for (_, old) in backups.into_iter().take(excess) {
            let _ = std::fs::remove_file(old);
        }
    }
    Some(dest.to_string_lossy().into_owned())
}

fn list_session_transcripts(name: &str) -> Vec<Value> {
    let dest_dir = transcripts_dir().join(name);
    let Ok(rd) = std::fs::read_dir(&dest_dir) else { return vec![] };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .filter_map(|(_, f)| {
            let md = f.metadata().ok()?;
            let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some(json!({
                "name": f.file_name()?.to_string_lossy(),
                "size": md.len(),
                "mtime": mtime,
            }))
        })
        .collect()
}

fn memory_shared_with(name: &str) -> Vec<String> {
    let wd = session_work_dir(name);
    if wd.is_empty() {
        return vec![];
    }
    let pname = project_name(&wd);
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("env"))
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
            .collect();
        names.sort();
        for other in names {
            if other == name {
                continue;
            }
            let owd = session_work_dir(&other);
            if !owd.is_empty() && project_name(&owd) == pname {
                out.push(other);
            }
        }
    }
    out
}

fn mem_inherit_files() -> Vec<String> {
    std::env::var("AMUX_MEMORY_INHERIT_FILES")
        .unwrap_or_else(|_| "CLAUDE.md".into())
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn add_inherited(out: &mut Vec<Value>, level: &str, path: &Path) {
    let exists = path.is_file();
    let (bytes, text) = if exists {
        let t = std::fs::read_to_string(path).unwrap_or_default();
        (t.len(), t)
    } else {
        (0, String::new())
    };
    out.push(json!({
        "level": level,
        "kind": "inherited",
        "path": path.to_string_lossy(),
        "exists": exists,
        "bytes": bytes,
        "text": text,
    }));
}

fn inherited_instruction_files(work_dir: &str, names: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    let home_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()));
    for n in names {
        add_inherited(&mut out, "user", &claude_home().join(n));
    }
    if work_dir.is_empty() {
        return out;
    }
    let wd = expanduser(work_dir).canonicalize().unwrap_or_else(|_| expanduser(work_dir));
    let mut chain = vec![wd.clone()];
    let mut cur = wd;
    loop {
        if cur == home_dir || cur.parent().is_none() || !cur.starts_with(&home_dir) {
            break;
        }
        let Some(parent) = cur.parent() else { break };
        cur = parent.to_path_buf();
        chain.push(cur.clone());
    }
    for d in chain.iter().rev() {
        for n in names {
            add_inherited(&mut out, "project", &d.join(n));
        }
    }
    out
}

fn no_board_re() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)^\s*\[no[_-]?board\]\s*").unwrap())
}

// ---------------------------------------------------------------------------
// HTTP layer. Two routes matching the retired proxy shape; dispatch mirrors
// Python's (method, action, subid) tree so unknown verbs 404/405 the same.
// ---------------------------------------------------------------------------

fn jresp(status: StatusCode, v: Value) -> Response {
    (status, Json(v)).into_response()
}
fn j200(v: Value) -> Response {
    jresp(StatusCode::OK, v)
}
fn not_found() -> Response {
    jresp(StatusCode::NOT_FOUND, json!({"error": "not found"}))
}

/// py:801 _UI_TOKEN — sha256("amux-ui-guard:" + AUTH_TOKEN)[:40].
fn ui_token(state: &AppState) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(format!("amux-ui-guard:{}", state.auth_token.clone().unwrap_or_default()).as_bytes());
    hex::encode(h.finalize()).chars().take(40).collect()
}

/// py:804 _session_destructive_allowed.
fn session_destructive_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    if matches!(
        std::env::var("AMUX_ALLOW_AGENT_SESSION_DELETE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    ) {
        return true;
    }
    headers
        .get("x-amux-ui-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == ui_token(state))
        .unwrap_or(false)
}

/// Origin header, py:15208 _hdr_worker precedence (X-Amux-Worker first,
/// legacy X-Amux-Session).
fn hdr_worker(headers: &HeaderMap) -> String {
    for k in ["x-amux-worker", "x-amux-session"] {
        if let Some(v) = headers.get(k).and_then(|v| v.to_str().ok()) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    String::new()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sessions/{name}", any(session_root_handler))
        .route("/api/sessions/{name}/{*verb}", any(session_verb_handler))
        // CANONICAL SPELLING, same dispatcher. `/api/sessions/*` is exempt from
        // the alias layer (aliases.rs: the bare list has a dedicated shape
        // handler and the verbs used to proxy to Python), so nothing was
        // rewriting `/api/workers/<n>/<verb>` onto these — the canonical name
        // for the verbs simply did not exist, and only the legacy one answered.
        //
        // That is not cosmetic: the INSTALLED `amux send` posts to
        // /api/workers/<n>/send. Against Python it worked; after the cutover it
        // got 405 and the CLI fell back to RAW TMUX KEYSTROKES — unstamped,
        // unaudited, delivery unverified. So every session's `amux send` lost
        // the origin stamp that AMUX-1768 exists to provide and that CLAUDE.md
        // instructs every session to rely on ("provenance comes from the server
        // stamp, not the text"). Two long inter-session messages were confirmed
        // LOST through that fallback the same afternoon.
        //
        // Fixed server-side rather than in the CLI deliberately: a CLI fix only
        // reaches machines that reinstall, while the route fixes every already
        // installed copy at once (ethos rule 1 — capability has to actually
        // reach everyone, not just exist).
        .route("/api/workers/{name}/{*verb}", any(session_verb_handler))
        // Long prompts ride /send bodies; axum's 2MB default is Python's cap
        // too (none), so disable rather than invent one.
        .layer(axum::extract::DefaultBodyLimit::disable())
}

async fn session_root_handler(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    method: Method,
    headers: HeaderMap,
    RawQuery(q): RawQuery,
    body: axum::body::Bytes,
) -> Response {
    dispatch(state, name, String::new(), method, headers, q, body).await
}

async fn session_verb_handler(
    State(state): State<AppState>,
    AxumPath((name, verb)): AxumPath<(String, String)>,
    method: Method,
    headers: HeaderMap,
    RawQuery(q): RawQuery,
    body: axum::body::Bytes,
) -> Response {
    dispatch(state, name, verb, method, headers, q, body).await
}

async fn dispatch(
    state: AppState,
    name: String,
    verb: String,
    method: Method,
    headers: HeaderMap,
    q: Option<String>,
    body_bytes: axum::body::Bytes,
) -> Response {
    // Rust-managed worker? Its verbs are the modern API's (kept from the
    // retired proxy's guard — a legacy-path call gets a pointer, never a
    // silent 404).
    let is_rust_worker = state
        .store
        .read()
        .ok()
        .and_then(|conn| crate::db::queries::get_worker(&conn, &name).ok().flatten())
        .is_some();
    if is_rust_worker {
        return jresp(
            StatusCode::NOT_IMPLEMENTED,
            json!({
                "error": "rust-managed worker — use /api/workers",
                "worker": name,
                "hint": format!("/api/workers/{name}"),
            }),
        );
    }
    // Python's route regex allows exactly action(/subid); deeper nesting 404s.
    let mut parts = verb.splitn(3, '/');
    let action = parts.next().unwrap_or("").to_string();
    let subid = parts.next().unwrap_or("").to_string();
    if parts.next().is_some() {
        return not_found();
    }
    let qs = parse_qs(q.as_deref().unwrap_or(""));
    let body: Value = match parse_body(&body_bytes) {
        Ok(v) => v,
        Err(e) => return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    };
    // Validate session exists (py:74882) — for every action, share included.
    // ONE exception: a RETRY of a partially-completed rename addresses the
    // OLD name after its env file already moved; admit it so the convergent
    // cascade can finish the remainder (owner addendum, AMUX-2598).
    if !env_path(&name).exists() {
        let rename_resume = method == Method::PATCH
            && action == "config"
            && body
                .get("rename")
                .and_then(|v| v.as_str())
                .map(|r| {
                    let target = sanitize_session_name(r);
                    !target.is_empty() && target != name && env_path(&target).exists()
                })
                .unwrap_or(false);
        if !rename_resume {
            return jresp(StatusCode::NOT_FOUND, json!({"error": format!("session '{name}' not found")}));
        }
    }

    // /share is its own family in Python (py:65953), any method.
    if action == "share" {
        return share_handler(&state, &name, &method, &headers, &body).await;
    }

    if method == Method::GET || method == Method::HEAD {
        return get_dispatch(&state, &name, &action, &subid, &qs).await;
    }
    if action == "tracked-files" && (method == Method::POST || method == Method::DELETE) {
        return tracked_files_mutate(&name, &method, &body);
    }
    if action == "steer" {
        return steer_mutate(&state, &name, &method, &headers, &body).await;
    }
    if method == Method::POST {
        return post_dispatch(&state, &name, &action, &headers, &body).await;
    }
    if method == Method::PATCH {
        return patch_dispatch(&state, &name, &action, &body).await;
    }
    jresp(StatusCode::METHOD_NOT_ALLOWED, json!({"error": "method not allowed"}))
}

fn qs_first<'a>(qs: &'a [(String, String)], key: &str, default: &'a str) -> &'a str {
    qs_get(qs, key).unwrap_or(default)
}
fn qs_flag(qs: &[(String, String)], key: &str) -> bool {
    matches!(qs_get(qs, key), Some("1") | Some("true") | Some("yes"))
}

// ---------------------------------------------------------------------------
// GET verbs (py:74887-75418).
// ---------------------------------------------------------------------------

async fn get_dispatch(
    state: &AppState,
    name: &str,
    action: &str,
    subid: &str,
    qs: &[(String, String)],
) -> Response {
    match action {
        "" => {
            // Bare GET → the SAME record the list endpoint serves (py:74892).
            let conn = match state.store.read() {
                Ok(c) => c,
                Err(e) => return jresp(StatusCode::SERVICE_UNAVAILABLE, json!({"error": e.to_string()})),
            };
            match crate::api::sessions_legacy::build_array(&conn) {
                Ok(arr) => {
                    match arr.into_iter().find(|x| x["name"] == json!(name)) {
                        Some(rec) => j200(rec),
                        None => jresp(
                            StatusCode::NOT_FOUND,
                            json!({"error": format!("session '{name}' not found")}),
                        ),
                    }
                }
                Err(e) => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
            }
        }
        "tasks" => j200(session_cc_tasks(name).await),
        "peek" => {
            let lines: i64 = qs_first(qs, "lines", "80").parse().unwrap_or(80);
            let live_only = qs_flag(qs, "live");
            let no_trim = qs_flag(qs, "notrim");
            j200(peek_response(name, lines, live_only, no_trim).await)
        }
        "transcript" => {
            let mx: usize = qs_first(qs, "max", "40000").parse().unwrap_or(40000);
            let txt = render_session_transcript(name, mx);
            if txt.is_empty() {
                j200(json!({"name": name, "output": "", "empty": true}))
            } else {
                j200(json!({"name": name, "output": txt, "source": "transcript"}))
            }
        }
        "info" => {
            // py:20461 get_session_info.
            let cfg = parse_env(name);
            let raw_dir = cfg.get_or("CC_DIR", "");
            let dir = if raw_dir.is_empty() { String::new() } else { work_dir_of(&cfg) };
            j200(json!({
                "name": name,
                "dir": dir,
                "desc": cfg.get_or("CC_DESC", ""),
                "pinned": cfg.get("CC_PINNED") == Some("1"),
                "tags": cfg.get_or("CC_TAGS", "").split(',').map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>(),
                "flags": cfg.get_or("CC_FLAGS", ""),
                "provider": cfg.get_or("CC_PROVIDER", "claude"),
                "running": is_running(name).await,
                "raw": std::fs::read_to_string(env_path(name)).unwrap_or_default(),
            }))
        }
        "instructions" => j200(json!({
            "name": name,
            "instructions": meta_str(&load_meta(name), "instructions").trim(),
        })),
        "dirty" => {
            let wd = session_work_dir(name);
            let files = if wd.is_empty() { vec![] } else { session_dirty_files(name, &wd).await };
            j200(json!({
                "name": name,
                "dirty": !files.is_empty(),
                "count": files.len(),
                "files": files.iter().take(50).collect::<Vec<_>>(),
            }))
        }
        "commit-guard" => {
            let cfg = parse_env(name);
            let per = cfg.get_or("AMUX_COMMIT_GUARD_SESSION", "").trim().to_lowercase();
            let global = !matches!(
                std::env::var("AMUX_COMMIT_GUARD").unwrap_or_else(|_| "1".into()).trim().to_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            );
            let override_v: Value = if per.is_empty() {
                Value::Null
            } else {
                json!(!matches!(per.as_str(), "0" | "false" | "off" | "no"))
            };
            let enabled = match &override_v {
                Value::Bool(b) => *b,
                _ => global,
            };
            j200(json!({"name": name, "enabled": enabled, "global": global, "override": override_v}))
        }
        "meta" => {
            // py:75162 — merged meta + env-derived fields.
            let cfg = parse_env(name);
            let meta = load_meta(name);
            let provider = provider_of(&cfg);
            let flags = cfg.get_or("CC_FLAGS", "");
            let env_mtime = env_path(name)
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mf = mem_file(name);
            let mem_size = mf.metadata().map(|m| m.len()).unwrap_or(0);
            let mut out = meta.clone();
            if !out.contains_key("creator") {
                out.insert("creator".into(), json!(cfg.get_or("CC_CREATOR", "")));
            }
            let configured = {
                let m = extract_model_from_flags(flags);
                if m.is_empty() { default_model_for_provider(&provider) } else { m }
            };
            out.insert("name".into(), json!(name));
            out.insert("dir".into(), json!(cfg.get_or("CC_DIR", "")));
            out.insert("provider".into(), json!(provider));
            out.insert("flags".into(), json!(flags));
            out.insert("configured_model".into(), json!(configured));
            out.insert("desc".into(), json!(cfg.get_or("CC_DESC", "")));
            out.insert(
                "tags".into(),
                json!(cfg.get_or("CC_TAGS", "").split(',').map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>()),
            );
            out.insert("env_updated".into(), json!(env_mtime));
            out.insert("mem_size".into(), json!(mem_size));
            out.insert("mem_path".into(), json!(mf.to_string_lossy()));
            j200(Value::Object(out))
        }
        "log" => log_get(name, subid, qs),
        "transcripts" => {
            if !subid.is_empty() {
                // Download one backup file.
                let tf = transcripts_dir().join(name).join(subid);
                if !tf.is_file() {
                    return not_found();
                }
                let Ok(data) = std::fs::read(&tf) else { return not_found() };
                return (
                    StatusCode::OK,
                    [
                        ("content-type", "application/x-ndjson".to_string()),
                        ("content-disposition", format!("attachment; filename=\"{subid}\"")),
                    ],
                    data,
                )
                    .into_response();
            }
            j200(json!({"transcripts": list_session_transcripts(name)}))
        }
        "tracked-files" => {
            let meta = load_meta(name);
            j200(json!({"files": meta.get("tracked_files").cloned().unwrap_or(json!([]))}))
        }
        "stats" => {
            let cfg = parse_env(name);
            j200(get_claude_stats(cfg.get_or("CC_DIR", "")))
        }
        "git" => git_get(name, subid, qs).await,
        "memory" => {
            let mf = mem_file(name);
            let content = std::fs::read_to_string(&mf).unwrap_or_default();
            let wd = session_work_dir(name);
            j200(json!({
                "content": content,
                "path": mf.to_string_lossy(),
                "work_dir": wd,
                "claude_project": if wd.is_empty() { String::new() } else { project_name(&wd) },
                "shared_with": memory_shared_with(name),
            }))
        }
        "memory-inherited" => {
            let wd = session_work_dir(name);
            let names: Vec<String> = qs_first(qs, "file", "")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            let effective = if names.is_empty() { mem_inherit_files() } else { names.clone() };
            let inh = inherited_instruction_files(&wd, &effective);
            let found: Vec<&Value> = inh.iter().filter(|l| l["exists"] == json!(true)).collect();
            let missing: Vec<Value> = inh
                .iter()
                .filter(|l| l["exists"] != json!(true))
                .map(|l| {
                    let mut m = l.as_object().cloned().unwrap_or_default();
                    m.remove("text");
                    Value::Object(m)
                })
                .collect();
            let total: u64 = found.iter().map(|l| l["bytes"].as_u64().unwrap_or(0)).sum();
            j200(json!({
                "worker": name,
                "dir": wd,
                "filenames": effective,
                "configured_by": "AMUX_MEMORY_INHERIT_FILES (server.env), or ?file= on this call",
                "note": "Loaded by Claude Code itself, not composed by amux — shown so the inheritance is visible, not duplicated into memory.",
                "found": found,
                "missing": missing,
                "total_bytes": total,
            }))
        }
        "search" => {
            let q = qs_first(qs, "q", "").trim().to_string();
            let lim_raw = qs_first(qs, "limit", "").trim().to_string();
            let lim: i64 = if lim_raw.is_empty() {
                0
            } else {
                lim_raw.parse::<i64>().map(|v| v.clamp(1, 2000)).unwrap_or(0)
            };
            let root = session_work_dir(name);
            if root.is_empty() {
                return j200(json!({
                    "session": name, "query": q, "root": "", "engine": "", "results": [],
                    "files": 0, "matches": 0, "truncated": false,
                    "searched_ignored": qs_flag(qs, "ignored"),
                    "searched_hidden": qs_flag(qs, "ignored"),
                    "limit": if lim != 0 { lim } else { 300 },
                    "error": "worker has no CC_DIR configured",
                }));
            }
            let literal = !matches!(qs_get(qs, "literal"), Some("0") | Some("false") | Some("no"));
            let case = qs_first(qs, "case", "smart").to_lowercase();
            let globs: Vec<String> =
                qs.iter().filter(|(k, v)| k == "glob" && !v.is_empty()).map(|(_, v)| v.clone()).collect();
            let mut out =
                crate::api::fs::fs_search(&root, &q, lim, literal, &case, qs_flag(qs, "ignored"), &globs).await;
            out.insert("session".into(), json!(name));
            let status = if out.get("error").and_then(|e| e.as_str()) == Some("missing query") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::OK
            };
            jresp(status, Value::Object(out))
        }
        "steer" => {
            let conn = match state.store.read() {
                Ok(c) => c,
                Err(e) => return jresp(StatusCode::SERVICE_UNAVAILABLE, json!({"error": e.to_string()})),
            };
            if qs_first(qs, "history", "0") == "1" {
                let mut out = vec![];
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT id, text, queued_at, delivered_at FROM steering_history \
                     WHERE session=? ORDER BY delivered_at DESC LIMIT 100",
                ) {
                    if let Ok(rows) = stmt.query_map([name], |r| {
                        Ok(json!({
                            "id": r.get::<_, String>(0)?,
                            "text": r.get::<_, String>(1)?,
                            "queued_at": r.get::<_, Option<f64>>(2)?,
                            "delivered_at": r.get::<_, f64>(3)?,
                        }))
                    }) {
                        out = rows.flatten().collect();
                    }
                }
                return j200(json!(out));
            }
            let mut out = vec![];
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, text, queued_at, COALESCE(guard,'') FROM steering_queue \
                 WHERE session=? ORDER BY queued_at ASC",
            ) {
                if let Ok(rows) = stmt.query_map([name], |r| {
                    Ok(json!({
                        "id": r.get::<_, String>(0)?,
                        "text": r.get::<_, String>(1)?,
                        "queued_at": r.get::<_, f64>(2)?,
                        "guard": r.get::<_, String>(3)?,
                    }))
                }) {
                    out = rows.flatten().collect();
                }
            }
            j200(json!(out))
        }
        "env-explain" | "memory-explain" => jresp(
            StatusCode::NOT_IMPLEMENTED,
            json!({
                "error": format!("{action} is not ported to the rust origin yet"),
                "python_source": "amux-server.py:74901 (env-explain) / 74957 (memory-explain)",
                "note": "layered env/memory composition is a named residual gap in api/session_verbs.rs",
            }),
        ),
        _ => not_found(),
    }
}

/// GET log + log/info (py:75187-75250).
fn log_get(name: &str, subid: &str, qs: &[(String, String)]) -> Response {
    let lp = log_path(name);
    let want_plain = matches!(
        qs_first(qs, "plain", "0").to_lowercase().as_str(),
        "1" | "true" | "yes"
    );
    if subid == "info" {
        if want_plain {
            return match write_plain_log(name) {
                None => j200(json!({"exists": false, "size": 0, "path": plain_log_path(name).to_string_lossy(), "plain": true})),
                Some((cp, size)) => {
                    let mtime = cp
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    j200(json!({"exists": true, "size": size, "mtime": mtime, "path": cp.to_string_lossy(), "plain": true}))
                }
            };
        }
        let Ok(md) = lp.metadata() else {
            return j200(json!({"exists": false, "size": 0, "path": lp.to_string_lossy()}));
        };
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return j200(json!({"exists": true, "size": md.len(), "mtime": mtime, "path": lp.to_string_lossy()}));
    }
    if !subid.is_empty() {
        return not_found();
    }
    let Ok(mut data) = std::fs::read(&lp) else {
        return jresp(StatusCode::NOT_FOUND, json!({"error": "no log"}));
    };
    let tail_kb: usize = qs_first(qs, "tail_kb", "0").parse::<usize>().unwrap_or(0).min(1024);
    let before_kb: usize = qs_first(qs, "before_kb", "0").parse::<usize>().unwrap_or(0);
    if before_kb > 0 {
        let keep = data.len().saturating_sub(before_kb * 1024);
        data.truncate(keep);
    }
    let pre_len = data.len();
    if tail_kb > 0 && data.len() > tail_kb * 1024 {
        data = data[data.len() - tail_kb * 1024..].to_vec();
        if let Some(nl) = data.iter().position(|b| *b == b'\n') {
            if nl < 4096 {
                data = data[nl + 1..].to_vec();
            }
        }
    }
    let remaining = pre_len - data.len();
    if want_plain {
        let text = collapse_blank_runs(&strip_ansi(&String::from_utf8_lossy(&data)));
        data = text.into_bytes();
    }
    (
        StatusCode::OK,
        [
            ("content-type", "text/plain; charset=utf-8".to_string()),
            ("content-disposition", format!("attachment; filename=\"{name}.log\"")),
            ("x-log-remaining", remaining.to_string()),
        ],
        data,
    )
        .into_response()
}

/// GET git (+ commits / commit-detail / diff), py:75277-75361. The
/// _install_amux_commit_hook side effect is not ported (Python still owns
/// hook installation during coexistence).
async fn git_get(name: &str, subid: &str, qs: &[(String, String)]) -> Response {
    let wd = session_work_dir(name);
    match subid {
        "commits" => {
            let count: i64 = qs_first(qs, "count", "30").parse().unwrap_or(30);
            let fmt = "%H%x00%an%x00%ai%x00%s%x00%b%x1E";
            let count_arg = format!("-{count}");
            let fmt_arg = format!("--format={fmt}");
            let mut commits = vec![];
            if let Some(out) = git_out(&wd, &["log", &count_arg, &fmt_arg], Duration::from_secs(10)).await {
                let sess_re = regex::Regex::new(r"(?m)^Amux-Session:\s*(.+)$").unwrap();
                for entry in out.split('\u{1E}') {
                    let entry = entry.trim();
                    if entry.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = entry.splitn(5, '\0').collect();
                    if parts.len() >= 4 {
                        let mut body_txt = parts.get(4).map(|s| s.trim().to_string()).unwrap_or_default();
                        let mut amux_sess = String::new();
                        if let Some(m) = sess_re.captures(&body_txt) {
                            amux_sess = m[1].trim().to_string();
                            body_txt = sess_re.replace_all(&body_txt, "").trim().to_string();
                        }
                        commits.push(json!({
                            "hash": parts[0], "author": parts[1], "date": parts[2],
                            "subject": parts[3], "body": body_txt, "amux_session": amux_sess,
                        }));
                    }
                }
            }
            j200(json!({"commits": commits}))
        }
        "commit-detail" => {
            let sha = qs_first(qs, "sha", "").to_string();
            if sha.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "sha required"}));
            }
            // Commit-ish only — prevents `--output=<path>` arbitrary writes.
            let ok_re =
                regex::Regex::new(r"^(?:[0-9a-fA-F]{4,64}|[A-Za-z0-9][A-Za-z0-9._/\-]{0,120})$").unwrap();
            if !ok_re.is_match(&sha) {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "invalid sha"}));
            }
            let show = git_out(&wd, &["show", &sha, "--stat", "--format=%H%n%an%n%ai%n%s%n%b%x00"], Duration::from_secs(10))
                .await
                .unwrap_or_default();
            let parts: Vec<&str> = show.splitn(2, '\0').collect();
            let meta: Vec<&str> = parts.first().map(|p| p.splitn(5, '\n').collect()).unwrap_or_default();
            let stat = parts.get(1).map(|s| s.trim().to_string()).unwrap_or_default();
            let diff = git_out(&wd, &["show", &sha, "--format="], Duration::from_secs(10)).await.unwrap_or_default();
            j200(json!({
                "hash": meta.first().copied().unwrap_or(sha.as_str()),
                "author": meta.get(1).copied().unwrap_or(""),
                "date": meta.get(2).copied().unwrap_or(""),
                "subject": meta.get(3).copied().unwrap_or(""),
                "body": meta.get(4).map(|s| s.trim()).unwrap_or(""),
                "stat": stat,
                "diff": diff,
            }))
        }
        "diff" => {
            let file_path = qs_first(qs, "file", "").to_string();
            let staged = qs_first(qs, "staged", "0") == "1";
            let base = qs_first(qs, "base", "").to_string();
            if !base.is_empty() {
                let base_re = regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._/\-]{0,120}$").unwrap();
                if !base_re.is_match(&base) {
                    return jresp(StatusCode::BAD_REQUEST, json!({"error": "invalid base"}));
                }
            }
            let mut args: Vec<String> = vec!["diff".into()];
            if !base.is_empty() {
                args.push(format!("{base}..HEAD"));
            } else if staged {
                args.push("--cached".into());
            }
            if !file_path.is_empty() {
                args.push("--".into());
                args.push(file_path.clone());
            }
            let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
            let diff = git_out(&wd, &args_ref, Duration::from_secs(10)).await.unwrap_or_default();
            j200(json!({"diff": diff, "file": file_path}))
        }
        "" => {
            let detail = qs_first(qs, "detail", "0") == "1";
            let cfg = parse_env(name);
            let mut info = git_info(&wd, detail).await;
            if detail {
                let sb = cfg.get_or("CC_BRANCH", "");
                info["session_branch"] = json!(if sb == "none" { "" } else { sb });
            }
            j200(info)
        }
        _ => not_found(),
    }
}

// ---------------------------------------------------------------------------
// tracked-files POST/DELETE (py:75419) — includes the conversation-id
// adoption guard (cross-link refusal).
// ---------------------------------------------------------------------------

fn conversation_owned_by_other(conv_id: &str, this_session: &str) -> String {
    if conv_id.is_empty() {
        return String::new();
    }
    if let Ok(rd) = std::fs::read_dir(sessions_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            let Some(fname) = p.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(other) = fname.strip_suffix(".meta.json") else { continue };
            if other == this_session {
                continue;
            }
            if meta_str(&load_meta(other), "cc_conversation_id") == conv_id {
                return other.to_string();
            }
        }
    }
    String::new()
}

fn tracked_files_mutate(name: &str, method: &Method, body: &Value) -> Response {
    let mut meta = load_meta(name);
    let mut tracked: Vec<String> = meta
        .get("tracked_files")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let files: Vec<String> = match body.get("files") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => vec![],
    };
    if *method == Method::POST {
        let conv_id = body_str(body, "conversation_id").trim().to_string();
        let conv_re = regex::Regex::new(r"^[0-9a-fA-F-]{8,64}$").unwrap();
        if !conv_id.is_empty() && conv_re.is_match(&conv_id) {
            let owner = conversation_owned_by_other(&conv_id, name);
            if owner.is_empty() {
                meta.insert("cc_conversation_id".into(), json!(conv_id));
            }
            // Owned by another session: refuse to adopt (cross-link guard,
            // py:75437) — silently, matching Python (it only logs).
        }
        let cwd = body_str(body, "cwd").trim().to_string();
        if !cwd.is_empty() && cwd.starts_with('/') {
            meta.insert("cc_cwd".into(), json!(cwd));
        }
        for fp in files {
            if !fp.is_empty() && !tracked.contains(&fp) {
                tracked.push(fp);
            }
        }
    } else {
        let remove: std::collections::BTreeSet<&String> = files.iter().collect();
        tracked.retain(|f| !remove.contains(f));
    }
    meta.insert("tracked_files".into(), json!(tracked));
    save_meta(name, &meta);
    j200(json!({"ok": true, "files": tracked}))
}

// ---------------------------------------------------------------------------
// steer POST/DELETE (py:75463-75533).
// ---------------------------------------------------------------------------

async fn steer_mutate(
    state: &AppState,
    name: &str,
    method: &Method,
    headers: &HeaderMap,
    body: &Value,
) -> Response {
    if *method == Method::DELETE {
        let msg_id = body_str(body, "id");
        let sent = body.get("sent").map(py_truthy).unwrap_or(false);
        let session = name.to_string();
        let id2 = msg_id.clone();
        let reply = state
            .store
            .write_async(move |conn| {
                ensure_fleet_tables(conn)?;
                let mut sent_row: Option<(String, f64)> = None;
                let removed: i64;
                if !id2.is_empty() {
                    sent_row = conn
                        .query_row(
                            "SELECT text, queued_at FROM steering_queue WHERE id=? AND session=?",
                            rusqlite::params![id2, session],
                            |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
                        )
                        .ok();
                    removed = conn.execute("DELETE FROM steering_queue WHERE id=?", [&id2])? as i64;
                } else {
                    removed = conn.execute("DELETE FROM steering_queue WHERE session=?", [&session])? as i64;
                }
                if let Some((text, queued_at)) = sent_row.filter(|_| sent) {
                    let hid = id2.clone();
                    conn.execute(
                        "INSERT OR REPLACE INTO steering_history(id, session, text, queued_at, delivered_at) VALUES(?,?,?,?,?)",
                        rusqlite::params![hid, session, redact_secrets(&text), queued_at, now_f64()],
                    )?;
                }
                // Smuggle the count through WriteReply.applied? No — recompute
                // is racy; return via a rev-free outcome and count separately.
                Ok(crate::db::WriteOutcome { applied: removed > 0, events: vec![] })
            })
            .await;
        return match reply {
            Ok(r) => j200(json!({"ok": true, "cleared": if r.applied { 1 } else { 0 }})),
            Err(e) => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
        };
    }
    if *method == Method::POST {
        let mut text = body_str(body, "text");
        if text.is_empty() {
            return jresp(StatusCode::BAD_REQUEST, json!({"error": "missing 'text'"}));
        }
        let client_id: String = body_str(body, "msg_id").trim().chars().take(64).collect();
        if !client_id.is_empty() && send_dedup_seen(state, name, &format!("steer:{client_id}")).await {
            return j200(json!({"ok": true, "deduped": true, "message": "duplicate retry ignored (already queued)"}));
        }
        // Strip [no-board] before ENQUEUE (AC-183): decide, then strip.
        let _skip_board = body.get("no_board").map(py_truthy).unwrap_or(false) || no_board_re().is_match(&text);
        if no_board_re().is_match(&text) {
            text = no_board_re().replace(&text, "").trim().to_string();
            if text.is_empty() {
                return jresp(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "message is empty after removing [no-board]"}),
                );
            }
        }
        let msg_id = steer_enqueue(state, name, &text, "").await;
        if body.get("record_history").map(py_truthy).unwrap_or(false) {
            let email = headers.get("x-amux-user-email").and_then(|v| v.to_str().ok()).unwrap_or("");
            cmd_hist_record(state, name, &text, "user", email).await;
            // Autotask/labelling: Python's model-call feature — gap named in
            // the module doc.
        }
        return j200(json!({"ok": true, "id": msg_id, "message": "queued for next turn boundary"}));
    }
    jresp(StatusCode::METHOD_NOT_ALLOWED, json!({"error": "method not allowed"}))
}

/// Python truthiness over JSON (mirrors api/mod.rs's private helper).
fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// ---------------------------------------------------------------------------
// POST verbs (py:75534-76326).
// ---------------------------------------------------------------------------

async fn post_dispatch(
    state: &AppState,
    name: &str,
    action: &str,
    headers: &HeaderMap,
    body: &Value,
) -> Response {
    match action {
        "transcripts" => match backup_session_jsonl(name, "manual") {
            Some(path) => j200(json!({"ok": true, "path": path})),
            None => j200(json!({"ok": false, "message": "nothing to backup"})),
        },
        "send" => send_post(state, name, headers, body).await,
        "instructions" => {
            let mut saved = false;
            if let Some(instr) = body.get("instructions") {
                let v = instr.as_str().unwrap_or("").trim().to_string();
                update_meta(name, &[("instructions", json!(v))]);
                saved = true;
            }
            let mut applied = false;
            if body.get("apply").map(py_truthy).unwrap_or(false) {
                let instr = meta_str(&load_meta(name), "instructions").trim().to_string();
                if !instr.is_empty() {
                    if is_running(name).await {
                        let _ = send_text(state, name, &instr, false).await;
                    } else {
                        let st2 = state.clone();
                        let n = name.to_string();
                        tokio::spawn(async move { send_after_ready(st2, n, instr, 60).await });
                    }
                    applied = true;
                }
            }
            j200(json!({
                "ok": true,
                "instructions": meta_str(&load_meta(name), "instructions").trim(),
                "saved": saved,
                "applied": applied,
            }))
        }
        "keys" => {
            let keys = body_str(body, "keys");
            if keys.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "missing 'keys'"}));
            }
            let (ok, msg) = send_keys_op(name, &keys).await;
            let code = if ok {
                update_meta(name, &[("last_send", json!(now_i64()))]);
                StatusCode::OK
            } else if msg == "not running" {
                StatusCode::CONFLICT
            } else if msg.contains("not in allowed set") {
                // 400 so the offline queue drops it instead of retrying
                // forever (py:75700, 2026-07-18).
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            jresp(code, json!({"ok": ok, "message": msg}))
        }
        "resize" => {
            let cols = body.get("cols").and_then(|v| v.as_i64());
            let rows = body.get("rows").and_then(|v| v.as_i64()).unwrap_or(50);
            let Some(cols) = cols else {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "cols/rows must be integers"}));
            };
            if cols == 0 {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "cols required"}));
            }
            if !is_running(name).await {
                return jresp(StatusCode::CONFLICT, json!({"ok": false, "message": "not running"}));
            }
            let (ok, msg) = resize_pane(name, cols, rows).await;
            j200(json!({"ok": true, "resized": ok, "message": msg}))
        }
        "agent-nav" => {
            let d = body_str(body, "dir").trim().to_string();
            if !matches!(d.as_str(), "up" | "down" | "main" | "index") {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "dir must be up|down|main|index"}));
            }
            let idx = body.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            if d == "index" && idx < 0 {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "index required for dir=index"}));
            }
            if !is_running(name).await {
                return jresp(StatusCode::CONFLICT, json!({"ok": false, "message": "not running"}));
            }
            let (ok, msg) = agent_nav(name, &d, idx).await;
            jresp(
                if ok { StatusCode::OK } else { StatusCode::CONFLICT },
                json!({"ok": ok, "viewing": if ok { msg.clone() } else { String::new() }, "message": msg}),
            )
        }
        "memory" => {
            let content = body_str(body, "content");
            let mf = mem_file(name);
            let _ = std::fs::create_dir_all(memory_dir());
            if std::fs::write(&mf, content).is_err() {
                return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write memory file"}));
            }
            // _write_claude_memory (symlink into ~/.claude/projects) is not
            // ported — Python owns the memory composition during coexistence.
            j200(json!({"ok": true}))
        }
        "git" => {
            let branch = body_str(body, "branch").trim().to_string();
            let create = body.get("create").map(py_truthy).unwrap_or(false)
                || (body.get("worktree").map(py_truthy).unwrap_or(false)
                    && body.get("create").map(py_truthy).unwrap_or(false));
            let wd = session_work_dir(name);
            if wd.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "session has no directory"}));
            }
            if branch.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "branch name required"}));
            }
            let re = regex::Regex::new(r"^[a-zA-Z0-9_./@\-]+$").unwrap();
            if !re.is_match(&branch) {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "invalid branch name"}));
            }
            let mut args: Vec<&str> = vec!["-C", &wd, "checkout"];
            if create {
                args.push("-b");
            }
            args.push(&branch);
            match run_cmd("git", &args, Duration::from_secs(10)).await {
                Some(o) if o.status.success() => j200(json!({"ok": true, "branch": branch})),
                Some(o) => {
                    let err = String::from_utf8_lossy(if o.stderr.is_empty() { &o.stdout } else { &o.stderr })
                        .trim()
                        .to_string();
                    jresp(StatusCode::BAD_REQUEST, json!({"ok": false, "error": err}))
                }
                None => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"ok": false, "error": "git timed out"})),
            }
        }
        "git-push" => {
            if !is_running(name).await {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "session not running — start it first"}));
            }
            let cfg = parse_env(name);
            let branch = cfg.get_or("CC_BRANCH", "").to_string();
            let msg = if !branch.is_empty() && branch != "none" {
                format!(
                    "Deploy now. Your branch is `{branch}`. Run these steps:\n\
                     1. `git stash` (if needed to allow checkout)\n\
                     2. `git checkout {branch}` and `git stash pop` (if stashed)\n\
                     3. IMPORTANT: Only stage files YOU changed in this session — do NOT use `git add -A`. Use `git add <specific files>` for each file you modified.\n\
                     4. `git commit` with a good commit message summarizing YOUR changes only\n\
                     5. `git checkout main && git pull --ff-only origin main`\n\
                     6. `git merge {branch}` (resolve conflicts if any)\n\
                     7. `git push origin main`\n\
                     8. `git checkout {branch}` (go back to your branch)\n\
                     Do all steps now. If any step fails, fix it and continue."
                )
            } else {
                "Deploy now. You are on `main`. Run these steps:\n\
                 1. `git pull --ff-only origin main`\n\
                 2. IMPORTANT: Only stage files YOU changed in this session — do NOT use `git add -A`. Use `git add <specific files>` for each file you modified. Review `git diff` and only add files related to your task.\n\
                 3. `git commit` with a good commit message summarizing YOUR changes only\n\
                 4. `git push origin main`\n\
                 Do all steps now. If any step fails, fix it and continue."
                    .to_string()
            };
            let _ = send_text(state, name, &msg, false).await;
            j200(json!({"ok": true, "message": "deploy instructions sent to session"}))
        }
        "start" => {
            // RESPOND BEFORE THE CHOREOGRAPHY (AMUX-2557): validations
            // inline, launch in the background, instant 202.
            let cfg = parse_env(name);
            let wd0 = cfg.get_or("CC_DIR", "").trim().to_string();
            if !wd0.is_empty() && !expanduser(&wd0).is_dir() {
                return jresp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"ok": false, "message": format!("work dir missing: {wd0}")}),
                );
            }
            let prompt = body_str(body, "prompt").trim().to_string();
            let st2 = state.clone();
            let n = name.to_string();
            tokio::spawn(async move {
                let (ok, msg) = start_session(&st2, &n, "", false).await;
                if ok {
                    if !prompt.is_empty() {
                        send_after_ready(st2.clone(), n.clone(), prompt, 30).await;
                    }
                } else {
                    // A background failure must still be SEEN (ethos rule 4).
                    emit_event(
                        &st2,
                        &n,
                        "session.start_failed",
                        Some(json!({"message": chars_truncate(&msg, 200)})),
                        None,
                        "api-start",
                    )
                    .await;
                }
            });
            let meta = load_meta(name);
            let resumed = !meta_str(&meta, "cc_session_name").is_empty()
                || !meta_str(&meta, "cc_conversation_id").is_empty();
            jresp(StatusCode::ACCEPTED, json!({"ok": true, "message": "starting", "resumed": resumed}))
        }
        "stop" => {
            let st2 = state.clone();
            let n = name.to_string();
            tokio::spawn(async move {
                let (ok, _msg) = stop_session(&n).await;
                if ok {
                    emit_event(&st2, &n, "session.stopped", None, None, "api-stop").await;
                    // _complete_session_board_issue is a deliberate no-op in
                    // Python (py:12727) — nothing to port.
                }
            });
            jresp(StatusCode::ACCEPTED, json!({"ok": true, "message": "stopping"}))
        }
        "clear" => {
            let ptq = pt(name);
            match tmux(&["clear-history", "-t", &ptq]).await {
                Some(_) => j200(json!({"ok": true, "message": "cleared"})),
                None => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"ok": false, "message": "tmux clear-history timed out"})),
            }
        }
        "duplicate" => {
            let new_name = body_str(body, "new_name").trim().to_string();
            if new_name.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "missing new_name"}));
            }
            let re = regex::Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
            let new_name = re.replace_all(&new_name, "-").into_owned();
            let new_file = env_path(&new_name);
            if new_file.exists() {
                return jresp(StatusCode::CONFLICT, json!({"error": format!("session '{new_name}' already exists")}));
            }
            if std::fs::copy(env_path(name), &new_file).is_err() {
                return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "copy failed"}));
            }
            j200(json!({"ok": true, "message": format!("duplicated as {new_name}")}))
        }
        "clone" => clone_post(state, name, body).await,
        "archive" => {
            if !session_destructive_allowed(state, headers) {
                return jresp(StatusCode::FORBIDDEN, json!({"error": "archiving a session must be initiated by a human in the dashboard; sessions/agents cannot archive sessions (set AMUX_ALLOW_AGENT_SESSION_DELETE=1 to allow automation)"}));
            }
            let cfg = parse_env(name);
            if cfg.get("CC_PINNED") == Some("1") && !is_session_blocked(name) {
                return jresp(StatusCode::FORBIDDEN, json!({"error": "cannot archive pinned session — unpin first"}));
            }
            let (ok, msg) = archive_session(state, name).await;
            jresp(
                if ok { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR },
                json!({"ok": ok, "message": msg}),
            )
        }
        "wake" => {
            let (ok, msg) = wake_session(state, name).await;
            jresp(
                if ok { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR },
                json!({"ok": ok, "message": msg}),
            )
        }
        "reset" => {
            let (ok, msg) = reset_session(state, name).await;
            jresp(
                if ok { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR },
                json!({"ok": ok, "message": msg}),
            )
        }
        "commit-report" => {
            // Attach the commit to the in-flight card (py:76233-76246). The
            // cross-session sweep notice (py:76008-76230) is a named gap.
            let sha: String = body_str(body, "sha").trim().chars().take(16).collect();
            let subj: String = body_str(body, "subject").trim().chars().take(140).collect();
            if sha.is_empty() {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": "sha required"}));
            }
            let session = name.to_string();
            let sha2 = sha.clone();
            let reply = state
                .store
                .write_async(move |conn| {
                    let row: Option<String> = conn
                        .query_row(
                            "SELECT id FROM issues WHERE session=? AND deleted IS NULL \
                             AND COALESCE(archived,0)=0 AND status IN ('doing','review') \
                             AND owner_type='agent' ORDER BY updated DESC LIMIT 1",
                            [&session],
                            |r| r.get(0),
                        )
                        .ok();
                    let Some(issue_id) = row else {
                        return Ok(crate::db::WriteOutcome { applied: false, events: vec![] });
                    };
                    let log: String = conn
                        .query_row("SELECT COALESCE(log,'') FROM issues WHERE id=?", [&issue_id], |r| r.get(0))
                        .unwrap_or_default();
                    let ts = chrono::Local::now().format("%H:%M");
                    let new_log = format!("{}\n`{ts}` commit {sha2} — {subj}", log.trim_end()).trim().to_string();
                    conn.execute(
                        "UPDATE issues SET log=?, rev=COALESCE(rev,0)+1, updated=? WHERE id=?",
                        rusqlite::params![new_log, now_i64(), issue_id],
                    )?;
                    Ok(crate::db::WriteOutcome {
                        applied: true,
                        events: vec![crate::db::PendingEvent {
                            entity_type: amux_core::revision::EntityType::Other("issue".into()),
                            entity_id: issue_id,
                            mutation: amux_core::revision::MutationKind::Updated,
                            payload: None,
                        }],
                    })
                })
                .await;
            match reply {
                Ok(r) if !r.applied => j200(json!({"ok": true, "attached": Value::Null})),
                Ok(_) => {
                    // Re-read the card id for the response (the write closure
                    // cannot return it through WriteReply).
                    let attached: Option<String> = state.store.read().ok().and_then(|conn| {
                        conn.query_row(
                            "SELECT id FROM issues WHERE session=? AND deleted IS NULL \
                             AND COALESCE(archived,0)=0 AND status IN ('doing','review') \
                             AND owner_type='agent' ORDER BY updated DESC LIMIT 1",
                            [name],
                            |r| r.get(0),
                        )
                        .ok()
                    });
                    j200(json!({"ok": true, "attached": attached, "sha": sha}))
                }
                Err(e) => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
            }
        }
        "report" => report_post(state, name, body).await,
        "apply-template" => {
            let re = regex::Regex::new(r"[^a-z0-9\-]").unwrap();
            let tmpl_id = re.replace_all(&body_str(body, "template_id"), "").into_owned();
            let work_dir = body_str(body, "dir").trim().to_string();
            let Some(tmpl_root) = templates_dir() else {
                return jresp(StatusCode::NOT_FOUND, json!({"error": "template not found"}));
            };
            let tmpl_path = tmpl_root.join(&tmpl_id);
            if tmpl_id.is_empty() || !tmpl_path.is_dir() {
                return jresp(StatusCode::NOT_FOUND, json!({"error": "template not found"}));
            }
            if !work_dir.is_empty() {
                let work = expanduser(&work_dir);
                let _ = std::fs::create_dir_all(&work);
                if let Ok(meta_text) = std::fs::read_to_string(tmpl_path.join("template.json")) {
                    if let Ok(meta) = serde_json::from_str::<Value>(&meta_text) {
                        for d in meta["dirs"].as_array().cloned().unwrap_or_default() {
                            if let Some(d) = d.as_str() {
                                let _ = std::fs::create_dir_all(work.join(d));
                            }
                        }
                    }
                }
                let claude_file = tmpl_path.join("CLAUDE.md");
                if claude_file.exists() {
                    let dest = work.join("CLAUDE.md");
                    if !dest.exists() {
                        if let Ok(t) = std::fs::read_to_string(&claude_file) {
                            let _ = std::fs::write(&dest, t);
                        }
                    }
                }
            }
            j200(json!({"ok": true}))
        }
        "delete" => delete_post(state, name, headers).await,
        _ => not_found(),
    }
}

/// Templates live beside amux-server.py (py:143 TEMPLATES_DIR). The rust
/// binary resolves them via AMUX_TEMPLATES_DIR, then the installed
/// amux-server.py symlink's parent, then ~/.amux/templates.
fn templates_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("AMUX_TEMPLATES_DIR") {
        let p = PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }
    let installed = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/bin/amux-server.py");
    if let Ok(target) = std::fs::canonicalize(&installed) {
        if let Some(parent) = target.parent() {
            let t = parent.join("templates");
            if t.is_dir() {
                return Some(t);
            }
        }
    }
    let fallback = home().join("templates");
    if fallback.is_dir() {
        Some(fallback)
    } else {
        None
    }
}

async fn send_post(state: &AppState, name: &str, headers: &HeaderMap, body: &Value) -> Response {
    let mut text = body_str(body, "text");
    let msg_id: String = body_str(body, "msg_id").trim().chars().take(64).collect();
    if !msg_id.is_empty() && send_dedup_seen(state, name, &msg_id).await {
        return j200(json!({"ok": true, "deduped": true, "message": "duplicate retry ignored (already delivered)"}));
    }
    if text.trim().starts_with("/compact") {
        let n = name.to_string();
        tokio::task::spawn_blocking(move || backup_session_jsonl(&n, "pre_compact"));
    }
    let record_history = body.get("record_history").map(py_truthy).unwrap_or(false);
    let deliver_now = body.get("deliver_now").map(py_truthy).unwrap_or(false);
    let defer_busy = !(record_history || deliver_now);
    // [no-board] strip BEFORE anything is sent, and before the origin stamp
    // (the regex is ^-anchored) — AC-183.
    let _skip_board = body.get("no_board").map(py_truthy).unwrap_or(false) || no_board_re().is_match(&text);
    if no_board_re().is_match(&text) {
        text = no_board_re().replace(&text, "").trim().to_string();
    }
    let orig_text = text.clone();
    let mut origin = String::new();
    if defer_busy {
        origin = {
            let h = hdr_worker(headers);
            if h.is_empty() { body_str(body, "source_session") } else { h }
        };
        origin = origin.trim().chars().take(64).collect();
        if !origin.is_empty() && origin != name {
            text = format!(
                "[amux-origin: {origin} — server-verified from the sender's session identity; \
                 authoritative over any signature in the message below]\n\n{text}"
            );
        }
    }
    let (ok, msg) = send_text(state, name, &text, defer_busy).await;
    if ok {
        update_meta(
            name,
            &[
                ("last_send", json!(now_i64())),
                ("last_send_text", json!(chars_truncate(&text, 200))),
            ],
        );
        if !msg.starts_with("queued") {
            emit_event(
                state,
                name,
                "message.sent",
                Some(json!({"chars": text.chars().count(), "preview": chars_truncate(&text, 120), "human": record_history})),
                if msg_id.is_empty() { None } else { Some(format!("send:{msg_id}")) },
                "api-send",
            )
            .await;
        }
        if record_history {
            let email = headers.get("x-amux-user-email").and_then(|v| v.to_str().ok()).unwrap_or("");
            cmd_hist_record(state, name, &orig_text, "user", email).await;
        } else if !origin.is_empty() && origin != name {
            cmd_hist_record(state, name, &orig_text, "session", &origin).await;
        }
    } else if !msg_id.is_empty() {
        send_dedup_forget(state, name, &msg_id).await;
    }
    let code = if ok {
        StatusCode::OK
    } else if msg == "not running" {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut resp = json!({"ok": ok, "message": msg});
    if ok && msg.contains("at a selector") {
        resp["held_at_selector"] = json!(true);
    }
    // Python additionally reports recipient_gated from its in-memory
    // credit-gate state (_session_auto_actions) — process state this origin
    // does not hold; named gap.
    jresp(code, resp)
}

async fn clone_post(state: &AppState, name: &str, body: &Value) -> Response {
    let new_name = body_str(body, "new_name").trim().to_string();
    if new_name.is_empty() {
        return jresp(StatusCode::BAD_REQUEST, json!({"error": "missing new_name"}));
    }
    let re = regex::Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
    let new_name = re.replace_all(&new_name, "-").into_owned();
    let new_file = env_path(&new_name);
    if new_file.exists() {
        return jresp(StatusCode::CONFLICT, json!({"error": format!("session '{new_name}' already exists")}));
    }
    if std::fs::copy(env_path(name), &new_file).is_err() {
        return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "copy failed"}));
    }
    let source_meta = load_meta(name);
    let session_id = {
        let sid = meta_str(&source_meta, "cc_conversation_id");
        if !sid.is_empty() {
            sid
        } else {
            // py:20480 _find_latest_session_id — newest jsonl with real turns.
            let cfg = parse_env(name);
            let wd = work_dir_of(&cfg);
            find_latest_session_id(&wd)
        }
    };
    let (ok, msg, method_used) = if !session_id.is_empty() {
        let (ok, msg) =
            start_session(state, &new_name, &format!("--resume {session_id} --fork-session"), true).await;
        (ok, msg, "resume")
    } else {
        let (ok, msg) = start_session(state, &new_name, "", false).await;
        (ok, msg, "scrollback")
    };
    if !ok {
        return jresp(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"ok": false, "message": format!("cloned config but failed to start: {msg}")}),
        );
    }
    if method_used == "scrollback" && is_running(name).await {
        sleep_ms(5000).await;
        let ptq = pt(name);
        let mut scrollback = String::new();
        if let Some(o) = run_cmd("tmux", &["capture-pane", "-t", &ptq, "-p", "-S", "-3000"], CAPTURE_TIMEOUT).await {
            let raw = String::from_utf8_lossy(&o.stdout).into_owned();
            let cleaned = strip_ansi(&raw);
            let mut lines: Vec<&str> = cleaned.lines().collect();
            while lines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
                lines.remove(0);
            }
            while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                lines.pop();
            }
            scrollback = lines.join("\n");
        }
        if !scrollback.is_empty() {
            if scrollback.chars().count() > 50000 {
                let chars: Vec<char> = scrollback.chars().collect();
                scrollback = chars[chars.len() - 50000..].iter().collect();
            }
            let prompt = format!(
                "This session was cloned from '{name}'. Below is the recent terminal output \
                 from that session. Please continue the work from where it left off.\n\n```\n{scrollback}\n```"
            );
            let _ = send_literal(&new_name, &prompt).await;
            sleep_ms(1000).await;
            send_key(&new_name, "Enter").await;
        }
    }
    j200(json!({"ok": true, "message": format!("cloned as {new_name} (method: {method_used})"), "started": ok}))
}

fn find_latest_session_id(work_dir: &str) -> String {
    if work_dir.is_empty() {
        return String::new();
    }
    let project_dir = claude_home().join("projects").join(project_name(work_dir));
    let Ok(rd) = std::fs::read_dir(&project_dir) else { return String::new() };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .filter_map(|p| p.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, f) in files {
        for entry in iter_jsonl_tail(&f, u64::MAX) {
            if matches!(entry["type"].as_str(), Some("user") | Some("assistant")) {
                return f.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            }
        }
    }
    String::new()
}

async fn delete_post(state: &AppState, name: &str, headers: &HeaderMap) -> Response {
    if !session_destructive_allowed(state, headers) {
        return jresp(StatusCode::FORBIDDEN, json!({"error": "deleting a session must be initiated by a human in the dashboard; sessions/agents cannot delete sessions (set AMUX_ALLOW_AGENT_SESSION_DELETE=1 in ~/.amux/server.env to allow automation)"}));
    }
    let cfg = parse_env(name);
    if cfg.get("CC_PINNED") == Some("1") && !is_session_blocked(name) {
        return jresp(StatusCode::FORBIDDEN, json!({"error": "cannot delete pinned session — unpin first"}));
    }
    if is_running(name).await {
        let _ = stop_session(name).await;
    }
    // Worktree cleanup (py:76300).
    if cfg.get("CC_WORKTREE") == Some("1") {
        let wt_repo = cfg.get_or("CC_WORKTREE_REPO", "").to_string();
        let wt_dir = cfg.get_or("CC_DIR", "").to_string();
        if !wt_repo.is_empty() && !wt_dir.is_empty() {
            let _ = run_cmd(
                "git",
                &["-C", &wt_repo, "worktree", "remove", "--force", &wt_dir],
                Duration::from_secs(15),
            )
            .await;
        }
    }
    // Python leaves the tmux session alive after stop (shell only); the env
    // removal below unregisters it from the fleet. Kill it too so a deleted
    // probe leaves no tmux corpse — Python's delete relies on the archived
    // reaper for that, which this origin does not run.
    kill_tmux_session(name).await;
    let _ = std::fs::remove_file(env_path(name));
    let _ = std::fs::remove_file(mem_file(name));
    let _ = std::fs::remove_file(meta_path(name));
    let _ = std::fs::remove_file(log_path(name));
    // DB-side per-session state (Python clears in-memory maps; the durable
    // equivalents here are the steering queue rows).
    let n = name.to_string();
    let _ = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            conn.execute("DELETE FROM steering_queue WHERE session=?", [&n])?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
    j200(json!({"ok": true, "message": "deleted"}))
}

/// POST report (py:76238-76265) — the D1 report endpoint: harness-reported
/// state into the SHARED prefs store Python reads at boot and
/// sessions_legacy reads live.
async fn report_post(state: &AppState, name: &str, body: &Value) -> Response {
    let st_raw = body_str(body, "state").trim().to_lowercase();
    let st = match st_raw.as_str() {
        "working" | "busy" => "active",
        "done" => "idle",
        "blocked" => "waiting",
        other => other,
    }
    .to_string();
    if !matches!(st.as_str(), "active" | "idle" | "waiting" | "error") {
        return jresp(
            StatusCode::BAD_REQUEST,
            json!({"error": format!("state must be one of active|idle|waiting|error (got '{st_raw}')")}),
        );
    }
    let src: String = {
        let s = body_str(body, "source");
        let s = if s.is_empty() { "hook".to_string() } else { s };
        s.chars().take(40).collect()
    };
    let detail: String = body_str(body, "detail").chars().take(200).collect();
    let name_s = name.to_string();
    let st2 = st.clone();
    let src2 = src.clone();
    let reply = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            let mut reports: Value = conn
                .query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| json!({}));
            let prev_state =
                reports[&name_s]["state"].as_str().unwrap_or("").to_string();
            // A HEARTBEAT MUST NOT RESURRECT A FINISHED TURN (AMUX-2538):
            // tool-hook only refreshes an already-active turn.
            if src2 == "tool-hook" && prev_state != "active" {
                return Ok(crate::db::WriteOutcome { applied: false, events: vec![] });
            }
            reports[&name_s] = json!({
                "state": st2, "detail": detail, "source": src2, "ts": now_f64(),
            });
            conn.execute(
                "INSERT INTO prefs(key, value) VALUES('session_reports', ?1) \
                 ON CONFLICT(key) DO UPDATE SET value=?1",
                [reports.to_string()],
            )?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;
    match reply {
        Ok(r) if !r.applied => {
            // Heartbeat ignored — report the stored state like Python.
            let prev = state
                .store
                .read()
                .ok()
                .and_then(|conn| {
                    conn.query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .ok()
                })
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .map(|v| v[name]["state"].as_str().unwrap_or("").to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into());
            j200(json!({"ok": true, "state": prev, "note": "heartbeat ignored — no active turn to refresh"}))
        }
        Ok(_) => j200(json!({"ok": true, "state": st})),
        Err(e) => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
    }
}

// ---------------------------------------------------------------------------
// PATCH verbs: commit-guard (py:76319) + config (py:76327-76755).
// ---------------------------------------------------------------------------

async fn patch_dispatch(state: &AppState, name: &str, action: &str, body: &Value) -> Response {
    match action {
        "commit-guard" => {
            let global = !matches!(
                std::env::var("AMUX_COMMIT_GUARD").unwrap_or_else(|_| "1".into()).trim().to_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            );
            let raw = body.get("enabled");
            let override_v = match raw {
                None | Some(Value::Null) => None,
                Some(v) => Some(py_truthy(v)),
            };
            let f = env_path(name);
            let mut cfg = parse_env(name);
            match override_v {
                None => cfg.remove("AMUX_COMMIT_GUARD_SESSION"),
                Some(b) => cfg.set("AMUX_COMMIT_GUARD_SESSION", if b { "1" } else { "0" }),
            }
            if cfg.write(&f).is_err() {
                return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
            }
            let enabled = override_v.unwrap_or(global);
            j200(json!({
                "ok": true, "enabled": enabled, "global": global,
                "override": override_v.map(Value::Bool).unwrap_or(Value::Null),
            }))
        }
        "config" => config_patch(state, name, body).await,
        _ => not_found(),
    }
}

/// The provider/model/effort/yolo restart choreography shared by four config
/// keys (py:76470-76500 and friends): stash the live conversation id, stop
/// for restart, start.
async fn restart_for_swap(state: &AppState, name: &str, provider: &str) -> bool {
    if provider == "claude" {
        // Python captures the LIVE conv id from process argv (py:20546
        // _live_conv_id). The argv walk is not ported; the meta id is what
        // start_session resumes from, so a stale meta id falls back to a
        // fresh --name start rather than resuming a neighbour's conversation.
        let cfg = parse_env(name);
        let wd = work_dir_of(&cfg);
        if meta_str(&load_meta(name), "cc_conversation_id").is_empty() {
            let sid = find_latest_session_id(&wd);
            if !sid.is_empty() {
                update_meta(name, &[("cc_conversation_id", json!(sid))]);
            }
        }
    }
    let _ = stop_session(name).await;
    kill_tmux_session(name).await;
    let (ok, _msg) = start_session(state, name, "", false).await;
    ok
}

// ---------------------------------------------------------------------------
// Rename — a CONVERGENT cascade, not a one-shot (py:76333-76432, upgraded per
// the owner addendum on AMUX-2598: "we should have some kind of idempotency
// for stuff like that under the hood").
//
// Design, in the addendum's three axes:
// 1. IDEMPOTENT + CONVERGENT: rename-to-self is an honest no-op (nothing
//    written, store rev unmoved — Invariant 37). Every step is
//    skip-if-already-done, and a RETRY of the same rename after a partial
//    failure (old env already moved, stragglers left) is admitted by
//    dispatch's resume exception and completes the remainder.
// 2. ATOMIC WHERE POSSIBLE, JOURNALED WHERE NOT: all DB reference
//    migrations run in ONE writer transaction. The fs/tmux steps cannot
//    join it, so the rename is journaled to session_events BEFORE the first
//    step (`session.rename.started` {old,new,resuming}) and confirmed after
//    (`session.renamed` {old,new,steps}) — a crash mid-cascade is
//    diagnosable from the journal (ethos rule 4), and a step failure
//    returns 500 NAMING the steps that completed.
// 3. COLLISION + CONCURRENCY: both-envs-exist → 409 (python parity, and the
//    one state a resume cannot disambiguate); concurrent renames serialize
//    on RENAME_LOCK; every success names the canonical `name` so callers
//    re-address.
//
// Beyond Python's cascade (issues/schedules/session_gates/saved_messages +
// steering queue/history), this also migrates rows Python ORPHANS on
// rename: share_tokens (share links died), cmd_history (Messages tab
// emptied), the prefs session_reports key (self-reported status lost, so
// the lane fell back to scrape-derived status), the transcripts backup dir
// and the plain-log mirror. Deliberately NOT migrated, named here rather
// than silently inherited: session_events rows (append-only audit — history
// keeps the name it happened under; the rename journal entry links the two)
// and send_dedup rows (600s TTL, self-expiring).
// ---------------------------------------------------------------------------

static RENAME_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn sanitize_session_name(raw: &str) -> String {
    let re = regex::Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
    re.replace_all(raw.trim(), "-").into_owned()
}

/// Move old→new if old exists and new doesn't; report the state either way.
/// An fs error is a hard failure (the caller returns 500 naming the steps).
fn move_if(old: &Path, new: &Path, label: &str, steps: &mut Vec<String>) -> Result<(), String> {
    if old.exists() && !new.exists() {
        std::fs::rename(old, new).map_err(|e| format!("{label}: rename failed: {e}"))?;
        steps.push(format!("{label}: moved"));
    } else if new.exists() {
        steps.push(format!("{label}: already at target"));
    } else {
        steps.push(format!("{label}: nothing to move"));
    }
    Ok(())
}

async fn rename_session(state: &AppState, name: &str, raw_new: &str) -> Response {
    let new_name = sanitize_session_name(raw_new);
    if new_name.is_empty() {
        return jresp(StatusCode::BAD_REQUEST, json!({"error": "invalid name"}));
    }
    if new_name == name {
        // Honest no-op: nothing written anywhere, rev unmoved (Invariant 37).
        return j200(json!({
            "ok": true, "noop": true, "name": name,
            "message": format!("already named {name} — nothing to do"),
        }));
    }
    let _serialize = RENAME_LOCK.lock().await;
    let old_env = env_path(name);
    let new_env = env_path(&new_name);
    let resuming = !old_env.exists() && new_env.exists();
    if old_env.exists() && new_env.exists() {
        return jresp(StatusCode::CONFLICT, json!({"error": format!("'{new_name}' already exists")}));
    }
    if !old_env.exists() && !new_env.exists() {
        return jresp(StatusCode::NOT_FOUND, json!({"error": format!("session '{name}' not found")}));
    }
    let work_dir = if resuming {
        parse_env(&new_name).get_or("CC_DIR", "").to_string()
    } else {
        parse_env(name).get_or("CC_DIR", "").to_string()
    };
    // JOURNAL FIRST: a crash mid-cascade must be diagnosable from the event
    // log, not discovered as orphaned cards weeks later (ethos rule 4).
    emit_event(
        state, name, "session.rename.started",
        Some(json!({"old": name, "new": new_name, "resuming": resuming})),
        None, "config-rename",
    )
    .await;
    let mut steps: Vec<String> = Vec::new();
    let fail = |steps: &[String], err: String| {
        jresp(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({
                "error": err,
                "old": name, "new": new_name.clone(),
                "steps_completed": steps,
                "retry": format!("re-run PATCH config {{\"rename\": \"{new_name}\"}} — every step is skip-if-done, the retry completes the remainder"),
            }),
        )
    };
    // 1. tmux — session-level rename, skip-if-done. Runs before the env
    //    moves so a failure here leaves the registry untouched.
    {
        let running = tmux_sessions_set().await;
        if running.contains(&tmux_name(name)) {
            let stq = st(name);
            let new_tmux = tmux_name(&new_name);
            match tmux(&["rename-session", "-t", &stq, &new_tmux]).await {
                Some(o) if o.status.success() => steps.push("tmux: renamed".into()),
                Some(o) => {
                    return fail(&steps, format!(
                        "tmux rename-session failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    ))
                }
                None => return fail(&steps, "tmux rename-session timed out".into()),
            }
        } else if running.contains(&tmux_name(&new_name)) {
            steps.push("tmux: already renamed".into());
        } else {
            steps.push("tmux: not running".into());
        }
    }
    // 2-6. Registry + per-session files, each convergent.
    if let Err(e) = move_if(&old_env, &new_env, "env", &mut steps) {
        return fail(&steps, e);
    }
    if let Err(e) = move_if(&mem_file(name), &mem_file(&new_name), "memory", &mut steps) {
        return fail(&steps, e);
    }
    if let Err(e) = move_if(&meta_path(name), &meta_path(&new_name), "meta", &mut steps) {
        return fail(&steps, e);
    }
    if let Err(e) = move_if(&log_path(name), &log_path(&new_name), "log", &mut steps) {
        return fail(&steps, e);
    }
    if let Err(e) = move_if(&plain_log_path(name), &plain_log_path(&new_name), "plain-log", &mut steps) {
        return fail(&steps, e);
    }
    if let Err(e) = move_if(
        &transcripts_dir().join(name),
        &transcripts_dir().join(&new_name),
        "transcript-backups",
        &mut steps,
    ) {
        return fail(&steps, e);
    }
    // 7. Claude project symlink repair (py:76354) — best-effort, reported.
    if !work_dir.is_empty() {
        let link = claude_home().join("projects").join(project_name(&work_dir)).join("memory/MEMORY.md");
        if link.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            let _ = std::fs::remove_file(&link);
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(mem_file(&new_name), &link);
            steps.push("claude-memory-symlink: repointed".into());
        }
    }
    // 8. Every DB reference, ONE transaction. Python's four tables may be
    //    absent on a fresh rust-only home — reported as absent, never a
    //    silent skip. UPDATE ... WHERE session=old is naturally convergent
    //    (a retry matches 0 rows).
    let counts: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let counts_c = counts.clone();
    let old_s = name.to_string();
    let new_s = new_name.clone();
    let db_result = state
        .store
        .write_async(move |conn| {
            ensure_fleet_tables(conn)?;
            let mut out = Vec::new();
            // Python's cascade (py:76375-76437). issues: active only —
            // historical/deleted keep the old name, matching Python.
            let python_tables: [(&str, &str); 4] = [
                ("issues", "UPDATE issues SET session=?1 WHERE session=?2 AND deleted IS NULL"),
                ("schedules", "UPDATE schedules SET session=?1 WHERE session=?2"),
                ("session_gates", "UPDATE session_gates SET session=?1 WHERE session=?2"),
                ("saved_messages", "UPDATE saved_messages SET session=?1 WHERE session=?2"),
            ];
            for (table, sql) in python_tables {
                match conn.execute(sql, rusqlite::params![new_s, old_s]) {
                    Ok(n) => out.push(format!("db.{table}: {n} row(s)")),
                    Err(_) => out.push(format!("db.{table}: table absent (fresh home)")),
                }
            }
            for table in ["steering_queue", "steering_history", "share_tokens", "cmd_history"] {
                let n = conn.execute(
                    &format!("UPDATE {table} SET session=?1 WHERE session=?2"),
                    rusqlite::params![new_s, old_s],
                )?;
                out.push(format!("db.{table}: {n} row(s)"));
            }
            // prefs session_reports is keyed by NAME inside a JSON blob —
            // Python orphans it and the renamed lane loses its self-reported
            // status until the next hook fires.
            let reports: Option<String> = conn
                .query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| r.get(0))
                .ok();
            if let Some(raw) = reports {
                if let Ok(mut v) = serde_json::from_str::<Value>(&raw) {
                    if let Some(obj) = v.as_object_mut() {
                        if let Some(rep) = obj.remove(&old_s) {
                            obj.insert(new_s.clone(), rep);
                            conn.execute(
                                "UPDATE prefs SET value=?1 WHERE key='session_reports'",
                                [v.to_string()],
                            )?;
                            out.push("prefs.session_reports: key migrated".into());
                        } else {
                            out.push("prefs.session_reports: no report under old name".into());
                        }
                    }
                }
            }
            *counts_c.lock().unwrap() = out;
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![crate::db::PendingEvent {
                    entity_type: amux_core::revision::EntityType::Other("issue".into()),
                    entity_id: new_s.clone(),
                    mutation: amux_core::revision::MutationKind::Updated,
                    payload: None,
                }],
            })
        })
        .await;
    if let Err(e) = db_result {
        return fail(&steps, format!("db reference migration failed (transaction rolled back): {e}"));
    }
    steps.extend(counts.lock().unwrap().iter().cloned());
    steps.push("db.session_events: audit rows keep the old name (deliberate — the rename journal entry links them)".into());
    // 9. Re-export AMUX_SESSION for future panes (py:76416) — best-effort;
    //    the RUNNING shell keeps its env until restart, same as Python.
    if is_running(&new_name).await {
        let stq = st(&new_name);
        let _ = tmux(&["setenv", "-t", &stq, "AMUX_SESSION", &new_name]).await;
        steps.push("tmux-env: AMUX_SESSION re-exported for future panes".into());
    }
    emit_event(
        state, &new_name, "session.renamed",
        Some(json!({"old": name, "new": new_name, "resuming": resuming, "steps": steps})),
        None, "config-rename",
    )
    .await;
    j200(json!({
        "ok": true,
        "name": new_name,
        "message": format!("renamed to {new_name}"),
        "resumed_partial": resuming,
        "steps": steps,
    }))
}

async fn config_patch(state: &AppState, name: &str, body: &Value) -> Response {
    if !body.is_object() {
        return jresp(StatusCode::BAD_REQUEST, json!({"error": "payload must be a JSON object"}));
    }
    let f = env_path(name);
    let mut cfg = parse_env(name);

    // Rename — convergent cascade with journaling (owner addendum on
    // AMUX-2598: "if we change a name of a worker nothing happens — we
    // should have some kind of idempotency for stuff like that under the
    // hood"). See rename_session below.
    if let Some(rename) = body.get("rename") {
        return rename_session(state, name, rename.as_str().unwrap_or("")).await;
    }

    // Change provider (py:76434).
    if let Some(pv) = body.get("provider") {
        let Some(pv) = pv.as_str() else {
            return jresp(StatusCode::BAD_REQUEST, json!({"error": "provider must be a string"}));
        };
        let provider_val = pv.trim().to_lowercase();
        if !SESSION_PROVIDERS.contains(&provider_val.as_str()) {
            return jresp(
                StatusCode::BAD_REQUEST,
                json!({"error": "provider must be 'claude', 'codex', or 'gemini'"}),
            );
        }
        let old_provider = provider_of(&cfg);
        if provider_val == old_provider {
            return j200(json!({"ok": true, "message": format!("provider already set to {provider_val}")}));
        }
        let current_flags = cfg.get_or("CC_FLAGS", "").to_string();
        let flags_no_model = match strip_model_from_flags(&current_flags) {
            Ok(v) => v,
            Err(e) => {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": format!("existing CC_FLAGS for session '{name}' is malformed ({e}); fix the .env file manually before updating the provider")}));
            }
        };
        let was_yolo = is_yolo_enabled(&current_flags, &cfg);
        let flags_no_yolo = strip_provider_yolo_flags(&flags_no_model);
        let default_model = default_model_for_provider(&provider_val);
        let mut flags = if flags_no_yolo.is_empty() {
            format!("--model {default_model}")
        } else {
            format!("--model {default_model} {flags_no_yolo}")
        };
        if was_yolo {
            flags = format!("{flags} {}", provider_yolo_flag(&provider_val)).trim().to_string();
            cfg.set("CC_AUTO_CONTINUE", "1");
        }
        cfg.set("CC_PROVIDER", &provider_val);
        cfg.set("CC_FLAGS", &flags);
        let was_running = is_running(name).await;
        if capture_log_tail_for_reload(name, "provider swap").await {
            mark_pending_log_reload(name, "provider swap");
        }
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        let restarted = if was_running { restart_for_swap(state, name, &old_provider).await } else { false };
        let suffix = if restarted { " (session restarted; log reload queued)" } else { "" };
        return j200(json!({"ok": true, "message": format!("provider set to {}{suffix}", provider_label(&provider_val))}));
    }

    // Change model (py:76496), with optional inline effort.
    if let Some(mv) = body.get("model") {
        let model_val = match validate_model_name(mv) {
            Ok(v) => v,
            Err(e) => return jresp(StatusCode::BAD_REQUEST, json!({"error": e})),
        };
        let flags_no_model = match strip_model_from_flags(cfg.get_or("CC_FLAGS", "")) {
            Ok(v) => v,
            Err(e) => {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": format!("existing CC_FLAGS for session '{name}' is malformed ({e}); fix the .env file manually before updating the model")}));
            }
        };
        let mut flags = if model_val.is_empty() {
            flags_no_model
        } else if flags_no_model.is_empty() {
            format!("--model {model_val}")
        } else {
            format!("--model {model_val} {flags_no_model}")
        };
        if let Some(ev) = body.get("effort") {
            let effort_val = match validate_effort(ev) {
                Ok(v) => v,
                Err(e) => return jresp(StatusCode::BAD_REQUEST, json!({"error": e})),
            };
            flags = match set_effort_flag(&flags, &effort_val) {
                Ok(v) => v,
                Err(e) => {
                    return jresp(StatusCode::BAD_REQUEST, json!({"error": format!("existing CC_FLAGS for session '{name}' is malformed ({e}); fix the .env file manually before updating effort")}));
                }
            };
        }
        cfg.set("CC_FLAGS", &flags);
        let current_provider = provider_of(&cfg);
        let was_running = is_running(name).await;
        // Python also clears its in-memory credit-limit flag here (AF-14) —
        // process state this origin does not hold.
        if capture_log_tail_for_reload(name, "model swap").await {
            mark_pending_log_reload(name, "model swap");
        }
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        let restarted = if was_running { restart_for_swap(state, name, &current_provider).await } else { false };
        let suffix = if restarted { " (session restarted; log reload queued)" } else { "" };
        return j200(json!({"ok": true, "message": format!("model set to {model_val}{suffix}")}));
    }

    // Change effort only (py:76570).
    if let Some(ev) = body.get("effort") {
        let effort_val = match validate_effort(ev) {
            Ok(v) => v,
            Err(e) => return jresp(StatusCode::BAD_REQUEST, json!({"error": e})),
        };
        let flags = match set_effort_flag(cfg.get_or("CC_FLAGS", ""), &effort_val) {
            Ok(v) => v,
            Err(e) => {
                return jresp(StatusCode::BAD_REQUEST, json!({"error": format!("existing CC_FLAGS for session '{name}' is malformed ({e}); fix the .env file manually before updating effort")}));
            }
        };
        cfg.set("CC_FLAGS", &flags);
        let current_provider = provider_of(&cfg);
        let was_running = is_running(name).await;
        if capture_log_tail_for_reload(name, "effort change").await {
            mark_pending_log_reload(name, "effort change");
        }
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        let restarted = if was_running { restart_for_swap(state, name, &current_provider).await } else { false };
        let suffix = if restarted { " (session restarted; log reload queued)" } else { "" };
        let shown = if effort_val.is_empty() { "default".to_string() } else { effort_val };
        return j200(json!({"ok": true, "message": format!("effort set to {shown}{suffix}")}));
    }

    // Toggle YOLO (py:76608).
    if body.get("toggle_yolo").map(py_truthy).unwrap_or(false)
        || body.get("toggle_auto_continue").map(py_truthy).unwrap_or(false)
    {
        let provider = provider_of(&cfg);
        let flags = cfg.get_or("CC_FLAGS", "").to_string();
        let enabled;
        let new_flags;
        if is_yolo_enabled(&flags, &cfg) {
            new_flags = strip_provider_yolo_flags(&flags);
            cfg.set("CC_AUTO_CONTINUE", "0");
            enabled = false;
        } else {
            new_flags = format!("{flags} {}", provider_yolo_flag(&provider)).trim().to_string();
            cfg.set("CC_AUTO_CONTINUE", "1");
            enabled = true;
        }
        cfg.set("CC_FLAGS", &new_flags);
        let was_running = is_running(name).await;
        if was_running && capture_log_tail_for_reload(name, "YOLO mode change").await {
            mark_pending_log_reload(name, "YOLO mode change");
        }
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        let restarted = if was_running { restart_for_swap(state, name, &provider).await } else { false };
        let state_word = if enabled { "enabled" } else { "disabled" };
        let suffix = if restarted { " (session restarted; log reload queued)" } else { "" };
        return j200(json!({"ok": true, "message": format!("yolo {state_word}{suffix}")}));
    }

    // Change directory (py:76646): hard restart in the new dir when running.
    if let Some(dv) = body.get("dir") {
        let new_dir = dv.as_str().unwrap_or("").trim().to_string();
        let old_dir = cfg.get_or("CC_DIR", "").to_string();
        cfg.set("CC_DIR", &new_dir);
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        if new_dir != old_dir && is_running(name).await {
            let st2 = state.clone();
            let n = name.to_string();
            tokio::spawn(async move {
                // py:76651 _restart_in_new_dir: hard-kill then start. The
                // graceful stop records the resumable name first.
                let _ = stop_session(&n).await;
                kill_tmux_session(&n).await;
                sleep_ms(2000).await;
                let _ = start_session(&st2, &n, "", false).await;
            });
            return j200(json!({"ok": true, "message": "directory updated — restarting session"}));
        }
        return j200(json!({"ok": true, "message": "directory updated"}));
    }

    // Task label override (py:76662).
    if let Some(ts) = body.get("task_summary") {
        update_meta(name, &[("task_summary", json!(ts.as_str().unwrap_or("").trim()))]);
        return j200(json!({"ok": true, "message": "task label updated"}));
    }

    // Description (py:76667).
    if let Some(dv) = body.get("desc") {
        cfg.set("CC_DESC", dv.as_str().unwrap_or("").trim());
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        return j200(json!({"ok": true, "message": "description updated"}));
    }

    // Toggle pin (py:76673).
    if body.get("toggle_pin").map(py_truthy).unwrap_or(false) {
        let now_pinned = cfg.get("CC_PINNED") == Some("1");
        cfg.set("CC_PINNED", if now_pinned { "" } else { "1" });
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        return j200(json!({"ok": true, "message": "pin toggled"}));
    }

    // Branch (py:76679).
    if let Some(bv) = body.get("branch") {
        cfg.set("CC_BRANCH", bv.as_str().unwrap_or("").trim());
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        return j200(json!({"ok": true, "message": "branch updated"}));
    }

    // Tags (py:76685). Python invalidates its sessions cache here; this
    // origin computes the list per request, so the write IS the refresh.
    if let Some(tv) = body.get("tags") {
        cfg.set("CC_TAGS", tv.as_str().unwrap_or("").trim());
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        return j200(json!({"ok": true, "message": "tags updated"}));
    }

    // MCP config (py:76731).
    if let Some(mv) = body.get("mcp") {
        let mcp_val = mv.as_str().unwrap_or("").trim().to_lowercase();
        if !mcp_val.is_empty() && mcp_val != "chrome" {
            return jresp(StatusCode::BAD_REQUEST, json!({"error": "mcp must be 'chrome' or '' (empty)"}));
        }
        cfg.set("CC_MCP", &mcp_val);
        if cfg.write(&f).is_err() {
            return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "could not write session env"}));
        }
        let msg = if mcp_val.is_empty() { "mcp disabled".to_string() } else { format!("mcp set to {mcp_val}") };
        return j200(json!({"ok": true, "message": format!("{msg} (restart session to apply)")}));
    }

    // New conversation (py:76741).
    if body.get("new_conversation").map(py_truthy).unwrap_or(false) {
        if is_running(name).await {
            return jresp(
                StatusCode::CONFLICT,
                json!({"error": "stop the session before starting a new conversation"}),
            );
        }
        let mut meta = load_meta(name);
        meta.remove("cc_conversation_id");
        save_meta(name, &meta);
        return j200(json!({"ok": true, "message": "conversation reset — next start will be a fresh conversation"}));
    }

    jresp(StatusCode::BAD_REQUEST, json!({"error": "nothing to update"}))
}

// ---------------------------------------------------------------------------
// share (py:65953-65999) — token CRUD over the shared share_tokens table.
// ---------------------------------------------------------------------------

async fn share_handler(
    state: &AppState,
    name: &str,
    method: &Method,
    headers: &HeaderMap,
    body: &Value,
) -> Response {
    match *method {
        Method::POST => {
            let perms = {
                let p = body_str(body, "perms");
                if p.is_empty() { "output".to_string() } else { p }
            };
            let expires_hours = body.get("expires_hours").and_then(|v| v.as_i64());
            let label = body_str(body, "label");
            let token = {
                // secrets.token_urlsafe(16) parity: 16 random bytes, base64url.
                use base64::Engine as _;
                let mut buf = [0u8; 16];
                getrandom_fill(&mut buf);
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
            };
            let now = now_i64();
            let expires_at = expires_hours.map(|h| now + h * 3600);
            let (t2, s2, p2, l2) = (token.clone(), name.to_string(), perms.clone(), label.clone());
            let reply = state
                .store
                .write_async(move |conn| {
                    ensure_fleet_tables(conn)?;
                    conn.execute(
                        "INSERT INTO share_tokens (token, session, perms, created_at, expires_at, label) VALUES (?,?,?,?,?,?)",
                        rusqlite::params![t2, s2, p2, now, expires_at, l2],
                    )?;
                    Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                })
                .await;
            if let Err(e) = reply {
                return jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()}));
            }
            let host = headers
                .get("x-forwarded-host")
                .or_else(|| headers.get("host"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("localhost:8822")
                .to_string();
            let scheme = if headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https")
                || !host.contains(':')
                || host.ends_with(":8822")
            {
                "https"
            } else {
                "http"
            };
            j200(json!({"token": token, "url": format!("{scheme}://{host}/s/{token}"), "expires_at": expires_at}))
        }
        Method::GET => {
            let conn = match state.store.read() {
                Ok(c) => c,
                Err(e) => return jresp(StatusCode::SERVICE_UNAVAILABLE, json!({"error": e.to_string()})),
            };
            let mut out = vec![];
            if let Ok(mut stmt) = conn.prepare(
                "SELECT token, perms, created_at, expires_at, label FROM share_tokens WHERE session=?",
            ) {
                if let Ok(rows) = stmt.query_map([name], |r| {
                    Ok(json!({
                        "token": r.get::<_, String>(0)?,
                        "perms": r.get::<_, String>(1)?,
                        "created_at": r.get::<_, i64>(2)?,
                        "expires_at": r.get::<_, Option<i64>>(3)?,
                        "label": r.get::<_, String>(4)?,
                    }))
                }) {
                    out = rows.flatten().collect();
                }
            }
            j200(json!(out))
        }
        Method::DELETE => {
            let token = body_str(body, "token");
            let s2 = name.to_string();
            let reply = state
                .store
                .write_async(move |conn| {
                    ensure_fleet_tables(conn)?;
                    if token.is_empty() {
                        conn.execute("DELETE FROM share_tokens WHERE session=?", [&s2])?;
                    } else {
                        conn.execute(
                            "DELETE FROM share_tokens WHERE token=? AND session=?",
                            rusqlite::params![token, s2],
                        )?;
                    }
                    Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                })
                .await;
            match reply {
                Ok(_) => j200(json!({"ok": true})),
                Err(e) => jresp(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
            }
        }
        _ => not_found(),
    }
}

/// Random bytes without a new dependency: /dev/urandom, falling back to a
/// time+pid hash (share tokens are convenience links, not crypto keys — but
/// urandom is present on every platform this server targets).
fn getrandom_fill(buf: &mut [u8]) {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() {
            return;
        }
    }
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(format!("{}-{}-{:?}", std::process::id(), now_f64(), std::time::Instant::now()).as_bytes());
    let d = h.finalize();
    let n = buf.len().min(d.len());
    buf[..n].copy_from_slice(&d[..n]);
}

// ---------------------------------------------------------------------------
// Tests — hermetic AMUX_HOME + temp store; no tmux, no live fleet. The env
// mutation is process-global, so everything shares one test fn per concern
// group behind a lock (same pattern as proxy_composition.rs).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::db::Store::open(&dir.path().join("t.db")).unwrap();
        (
            AppState {
                store: std::sync::Arc::new(store),
                started: std::time::Instant::now(),
                build_hash: "test".into(),
                auth_token: None,
            },
            dir,
        )
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, v)
    }

    #[test]
    fn env_file_roundtrip_preserves_order_and_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.env");
        std::fs::write(&p, "# updated: old\nCC_DIR=\"/tmp/a b\"\nCC_TAGS='x, y'\nCC_DESC=plain\n").unwrap();
        let mut e = EnvFile::load(&p);
        assert_eq!(e.get("CC_DIR"), Some("/tmp/a b"));
        assert_eq!(e.get("CC_TAGS"), Some("x, y"));
        assert_eq!(e.get("CC_DESC"), Some("plain"));
        e.set("CC_NEW", "v");
        e.write(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // Key order preserved, new key appended, header present.
        let d = text.find("CC_DIR").unwrap();
        let t = text.find("CC_TAGS").unwrap();
        let n = text.find("CC_NEW").unwrap();
        assert!(d < t && t < n, "{text}");
        assert!(text.starts_with("# updated: "), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn flag_helpers_match_python_semantics() {
        // strip --model both forms, preserve the rest, re-quoted.
        assert_eq!(strip_model_from_flags("--model opus --effort high").unwrap(), "--effort high");
        assert_eq!(strip_model_from_flags("--model=opus -x").unwrap(), "-x");
        // The [1m] model ids must round-trip shell-safe (py:22735 rationale).
        let f = shell_quote_flags("--model claude-opus-4-6[1m]");
        assert_eq!(f, "--model 'claude-opus-4-6[1m]'");
        assert_eq!(extract_model_from_flags(&f), "claude-opus-4-6[1m]");
        // Unbalanced quote errs (never silently wipes flags).
        assert!(strip_model_from_flags("--model 'oops").is_err());
        // effort set/clear.
        assert_eq!(set_effort_flag("--model opus", "high").unwrap(), "--model opus --effort high");
        assert_eq!(set_effort_flag("--model opus --effort low", "").unwrap(), "--model opus");
        // yolo strip covers --approval-mode yolo.
        assert_eq!(strip_provider_yolo_flags("--yolo --model auto"), "--model auto");
        assert_eq!(strip_provider_yolo_flags("--approval-mode yolo -x"), "-x");
    }

    #[test]
    fn detectors_read_real_frames() {
        let claude_idle = "some output\n\u{276f} \n  ⏵⏵ bypass permissions on (shift+tab to cycle)";
        assert!(claude_ui_visible(claude_idle));
        assert!(!at_shell_prompt(claude_idle));
        let shell = "Last login: Sat\nmixpeek$ ";
        assert!(!claude_ui_visible(shell));
        assert!(at_shell_prompt(shell));
        // Spinner = active; prompt-glyph lines never count as chrome.
        let active = "\u{273b} Crunching\u{2026} (12s)\n\u{276f} typed text";
        assert_eq!(detect_claude_status(active), "active");
        let echoed = "\u{276f} [amux] VERIFY \u{2014} y\u{2026}\nmixpeek$ ";
        assert_ne!(detect_claude_status(echoed), "active");
        // Resume picker needs the ⌕ search glyph.
        assert!(at_resume_picker("Resume Session \u{2315}\nEnter to select"));
        assert!(!at_resume_picker("Enter to select"));
    }

    #[test]
    fn peek_text_utils_hold_their_contracts() {
        assert_eq!(collapse_blank_runs("a\n\n\n\nb"), "a\n\nb");
        let noise = "unset CLAUDECODE CLAUDE_CODE_ENTRYPOINT; cd /x; claude --model sonnet --dangerously-skip-permissions --name s\nreal content";
        assert_eq!(strip_launch_noise(noise), "real content");
        // All-scaffolding frame returns unchanged, never blanks the peek.
        let only_noise = "claude --resume abc --name s";
        assert_eq!(strip_launch_noise(only_noise), only_noise);
        assert_eq!(
            strip_scroll_pill("before Jump to bottom (click) ↓ after"),
            "before after"
        );
        // trim_live_overlap: ≥3 matching lines trims through the last one.
        let transcript = "alpha line one long enough\nbeta line two long enough\ngamma line three long enough";
        let live = format!("{transcript}\nfresh tail");
        assert_eq!(trim_live_overlap(transcript, &live), "fresh tail");
        // <3 matches keeps the frame whole.
        assert_eq!(trim_live_overlap("only one line here long enough", "x\ny"), "x\ny");
    }

    #[test]
    fn transcript_md_render_basics() {
        let out = md_to_ansi("# Head\n**bold** and `code`");
        assert!(out.contains("\x1b[1mHead\x1b[22m"), "{out:?}");
        assert!(out.contains("\x1b[1mbold\x1b[22m"), "{out:?}");
        assert!(out.contains("\x1b[38;5;153mcode"), "{out:?}");
        let table = md_to_ansi("| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(table.contains('\u{250c}') && table.contains('\u{2502}'), "{table}");
    }

    #[test]
    fn redaction_matches_python_pattern_family() {
        assert_eq!(redact_secrets("key sk-ant-abc123XYZ done"), "key SECRET_REDACTED done");
        assert_eq!(redact_secrets("mxp_sk_deadbeef"), "mxp_sk_REDACTED");
        assert_eq!(
            redact_secrets("ANTHROPIC_API_KEY=sk-live-x y"),
            "ANTHROPIC_API_KEY=REDACTED y"
        );
    }

    /// The file/DB-backed verbs, exercised through the full router shape on a
    /// hermetic fleet home — the same dispatch the live composition mounts.
    #[tokio::test]
    async fn file_backed_verbs_roundtrip_hermetically() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
        std::fs::write(
            home.path().join("sessions/probe.env"),
            "CC_DIR=\"/tmp\"\nCC_DESC=\"a probe\"\nCC_TAGS=\"alpha, beta\"\nCC_FLAGS=\"--model sonnet\"\n",
        )
        .unwrap();
        // Shared AMUX_HOME guard (settings::test_env): the var is
        // process-global and other lib tests set it too — an unserialized
        // set_var raced them and read another test's home mid-assert.
        let _home = crate::api::settings::test_env::set_home(home.path());
        let (state, _dir) = state();
        let app: Router = routes().with_state(state);

        // 404 for a missing session, Python's exact error shape.
        let (st, v) = call(&app, "GET", "/api/sessions/nope/meta", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], json!("session 'nope' not found"));

        // meta merges env-derived fields.
        let (st, v) = call(&app, "GET", "/api/sessions/probe/meta", None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["name"], json!("probe"));
        assert_eq!(v["provider"], json!("claude"));
        assert_eq!(v["configured_model"], json!("sonnet"));
        assert_eq!(v["tags"], json!(["alpha", "beta"]));
        assert_eq!(v["desc"], json!("a probe"));

        // info carries the raw env text.
        let (st, v) = call(&app, "GET", "/api/sessions/probe/info", None).await;
        assert_eq!(st, StatusCode::OK);
        assert!(v["raw"].as_str().unwrap().contains("CC_DESC"));
        assert_eq!(v["pinned"], json!(false));

        // config PATCH: desc, tags, pin, branch, mcp validation, task_summary.
        let (st, v) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"desc": "new desc"}))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(parse_env("probe").get("CC_DESC"), Some("new desc"));
        let (st, _) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"tags": "x, y"}))).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(parse_env("probe").get("CC_TAGS"), Some("x, y"));
        let (st, _) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"toggle_pin": true}))).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(parse_env("probe").get("CC_PINNED"), Some("1"));
        let (st, v) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"mcp": "bogus"}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        let (st, v) = call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({}))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], json!("nothing to update"));
        // model swap on a NOT-RUNNING session rewrites flags without restart.
        let (st, v) = call(
            &app,
            "PATCH",
            "/api/sessions/probe/config",
            Some(json!({"model": "opus", "effort": "high"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let flags = parse_env("probe").get("CC_FLAGS").unwrap().to_string();
        assert!(flags.contains("--model opus") && flags.contains("--effort high"), "{flags}");
        assert_eq!(v["message"], json!("model set to opus"));
        // yolo toggle writes the provider flag + CC_AUTO_CONTINUE.
        let (st, v) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"toggle_yolo": true}))).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let cfg = parse_env("probe");
        assert!(cfg.get_or("CC_FLAGS", "").contains("--dangerously-skip-permissions"));
        assert_eq!(cfg.get("CC_AUTO_CONTINUE"), Some("1"));

        // instructions save + read back.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/instructions",
            Some(json!({"instructions": "stay on task"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["saved"], json!(true));
        let (_, v) = call(&app, "GET", "/api/sessions/probe/instructions", None).await;
        assert_eq!(v["instructions"], json!("stay on task"));

        // tracked-files add/list/remove; conversation-id adoption guard.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/tracked-files",
            Some(json!({"files": ["a.rs", "b.rs"], "conversation_id": "12345678-abcd"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["files"], json!(["a.rs", "b.rs"]));
        assert_eq!(meta_str(&load_meta("probe"), "cc_conversation_id"), "12345678-abcd");
        let (_, v) = call(&app, "GET", "/api/sessions/probe/tracked-files", None).await;
        assert_eq!(v["files"], json!(["a.rs", "b.rs"]));
        let (_, v) = call(
            &app,
            "DELETE",
            "/api/sessions/probe/tracked-files",
            Some(json!({"files": ["a.rs"]})),
        )
        .await;
        assert_eq!(v["files"], json!(["b.rs"]));
        // A sibling claiming the same conversation must NOT be adopted.
        std::fs::write(home.path().join("sessions/sib.env"), "CC_DIR=\"/tmp\"\n").unwrap();
        std::fs::write(
            home.path().join("sessions/sib.meta.json"),
            json!({"cc_conversation_id": "99999999-aaaa"}).to_string(),
        )
        .unwrap();
        let (_, _) = call(
            &app,
            "POST",
            "/api/sessions/probe/tracked-files",
            Some(json!({"files": [], "conversation_id": "99999999-aaaa"})),
        )
        .await;
        assert_eq!(
            meta_str(&load_meta("probe"), "cc_conversation_id"),
            "12345678-abcd",
            "cross-link guard must refuse adopting a sibling's conversation"
        );

        // steer enqueue → visible in GET → delete clears.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/steer",
            Some(json!({"text": "queued message"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let id = v["id"].as_str().unwrap().to_string();
        let (_, v) = call(&app, "GET", "/api/sessions/probe/steer", None).await;
        assert_eq!(v[0]["text"], json!("queued message"));
        assert_eq!(v[0]["id"], json!(id));
        // Identical text replaces, never stacks (dedup-on-enqueue).
        let (_, _) = call(
            &app,
            "POST",
            "/api/sessions/probe/steer",
            Some(json!({"text": "queued message"})),
        )
        .await;
        let (_, v) = call(&app, "GET", "/api/sessions/probe/steer", None).await;
        assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
        // [no-board]-only message is a 400, not an empty enqueue.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/steer",
            Some(json!({"text": "[no-board]"})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        let (_, v) = call(&app, "DELETE", "/api/sessions/probe/steer", Some(json!({}))).await;
        assert_eq!(v["ok"], json!(true));
        let (_, v) = call(&app, "GET", "/api/sessions/probe/steer", None).await;
        assert_eq!(v.as_array().unwrap().len(), 0);

        // duplicate copies the env under a sanitized name.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/duplicate",
            Some(json!({"new_name": "probe copy!"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert!(env_path("probe-copy-").exists());
        let (st, _) = call(
            &app,
            "POST",
            "/api/sessions/probe/duplicate",
            Some(json!({"new_name": "probe copy!"})),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);

        // share token CRUD.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/share",
            Some(json!({"perms": "output", "expires_hours": 1, "label": "t"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let token = v["token"].as_str().unwrap().to_string();
        assert!(v["url"].as_str().unwrap().contains(&token));
        assert!(v["expires_at"].as_i64().unwrap() > now_i64());
        let (_, v) = call(&app, "GET", "/api/sessions/probe/share", None).await;
        assert_eq!(v[0]["token"], json!(token));
        let (_, v) =
            call(&app, "DELETE", "/api/sessions/probe/share", Some(json!({"token": token}))).await;
        assert_eq!(v["ok"], json!(true));
        let (_, v) = call(&app, "GET", "/api/sessions/probe/share", None).await;
        assert_eq!(v.as_array().unwrap().len(), 0);

        // report: state write, alias mapping, tool-hook heartbeat rule.
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/report",
            Some(json!({"state": "working", "source": "prompt-hook"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["state"], json!("active"));
        let (_, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/report",
            Some(json!({"state": "done", "source": "stop-hook"})),
        )
        .await;
        assert_eq!(v["state"], json!("idle"));
        // A tool-hook heartbeat must NOT resurrect the finished turn.
        let (_, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/report",
            Some(json!({"state": "active", "source": "tool-hook"})),
        )
        .await;
        assert_eq!(v["state"], json!("idle"), "{v}");
        assert!(v["note"].as_str().unwrap().contains("heartbeat ignored"));
        let (st, v) = call(
            &app,
            "POST",
            "/api/sessions/probe/report",
            Some(json!({"state": "bogus"})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

        // commit-guard PATCH: set/clear override.
        let (_, v) = call(
            &app,
            "PATCH",
            "/api/sessions/probe/commit-guard",
            Some(json!({"enabled": false})),
        )
        .await;
        assert_eq!(v["enabled"], json!(false));
        assert_eq!(parse_env("probe").get("AMUX_COMMIT_GUARD_SESSION"), Some("0"));
        let (_, v) = call(
            &app,
            "PATCH",
            "/api/sessions/probe/commit-guard",
            Some(json!({"enabled": null})),
        )
        .await;
        assert_eq!(v["override"], Value::Null);
        assert_eq!(parse_env("probe").get("AMUX_COMMIT_GUARD_SESSION"), None);

        // delete without the UI token is a 403 (destructive guard); with the
        // automation override it removes the env file.
        let (st, v) = call(&app, "POST", "/api/sessions/probe/delete", None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
        std::env::set_var("AMUX_ALLOW_AGENT_SESSION_DELETE", "1");
        // Pinned guard fires first.
        let (st, v) = call(&app, "POST", "/api/sessions/probe/delete", None).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{v}");
        let (_, _) =
            call(&app, "PATCH", "/api/sessions/probe/config", Some(json!({"toggle_pin": true}))).await;
        let (st, v) = call(&app, "POST", "/api/sessions/probe/delete", None).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert!(!env_path("probe").exists());
        std::env::remove_var("AMUX_ALLOW_AGENT_SESSION_DELETE");

        // Unknown verb 404s; unknown method 405s.
        let (st, _) = call(&app, "GET", "/api/sessions/sib/definitely-not", None).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        let (st, _) = call(&app, "PUT", "/api/sessions/sib/config", Some(json!({}))).await;
        assert_eq!(st, StatusCode::METHOD_NOT_ALLOWED);

    }
    /// The owner-addendum rename matrix: noop, happy-path cascade with
    /// attached rows, retry-after-partial convergence, target collision.
    #[tokio::test]
    async fn rename_is_convergent_journaled_and_collision_safe() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("sessions")).unwrap();
        let _home = crate::api::settings::test_env::set_home(home.path());
        let (state, _dir) = state();
        // The baseline migration carries the full Python schema, so the
        // attached rows use the REAL issues/schedules tables — the cascade
        // must carry them to the new name.
        state
            .store
            .write_async(|conn| {
                conn.execute_batch(
                    "INSERT INTO issues (id, title, session, status, owner_type, created, updated)
                        VALUES ('I-1', 'card', 'rn-old', 'doing', 'agent', 1, 1);
                     INSERT INTO schedules (id, title, session, command, created, updated)
                        VALUES ('S-1', 'sched', 'rn-old', 'noop', 1, 1);",
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();
        std::fs::write(env_path("rn-old"), "CC_DESC=\"lane\"\n").unwrap();
        std::fs::write(meta_path("rn-old"), json!({"instructions": "keep"}).to_string()).unwrap();
        std::fs::create_dir_all(logs_dir()).unwrap();
        std::fs::write(log_path("rn-old"), "log body\n").unwrap();
        let app: Router = routes().with_state(state.clone());

        // 1. Rename-to-self: honest no-op — nothing written, rev unmoved.
        let rev_before = state.store.current_rev().unwrap();
        let (st, v) = call(
            &app, "PATCH", "/api/sessions/rn-old/config", Some(json!({"rename": "rn-old"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["noop"], json!(true));
        assert_eq!(v["name"], json!("rn-old"));
        assert_eq!(state.store.current_rev().unwrap(), rev_before, "noop must not move the rev");

        // 2. Happy path: files move, attached board card + schedule +
        //    steering + self-report follow, response names the steps.
        let (_, _) = call(
            &app, "POST", "/api/sessions/rn-old/steer", Some(json!({"text": "queued"})),
        )
        .await;
        let (_, _) = call(
            &app, "POST", "/api/sessions/rn-old/report",
            Some(json!({"state": "idle", "source": "stop-hook"})),
        )
        .await;
        let (st, v) = call(
            &app, "PATCH", "/api/sessions/rn-old/config", Some(json!({"rename": "rn-new"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["name"], json!("rn-new"));
        assert!(!env_path("rn-old").exists() && env_path("rn-new").exists());
        assert!(meta_path("rn-new").exists() && log_path("rn-new").exists());
        let steps = v["steps"].to_string();
        assert!(steps.contains("db.issues: 1 row(s)"), "{steps}");
        assert!(steps.contains("db.schedules: 1 row(s)"), "{steps}");
        assert!(steps.contains("db.steering_queue: 1 row(s)"), "{steps}");
        assert!(steps.contains("prefs.session_reports: key migrated"), "{steps}");
        {
            let conn = state.store.read().unwrap();
            let sess: String = conn
                .query_row("SELECT session FROM issues WHERE id='I-1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sess, "rn-new");
            let sched: String = conn
                .query_row("SELECT session FROM schedules WHERE id='S-1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sched, "rn-new");
            // Journal: started + completed events both present (rule 4).
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM session_events WHERE type IN ('session.rename.started','session.renamed')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(n >= 2, "rename must journal start+finish, found {n}");
        }

        // 3. Retry-after-partial: simulate a crash that moved ONLY the env
        //    file, leaving meta/log/DB under the old name — the retry of the
        //    SAME rename converges the stragglers.
        std::fs::write(env_path("rn2-old"), "CC_DESC=\"lane2\"\n").unwrap();
        std::fs::write(meta_path("rn2-old"), json!({"instructions": "x"}).to_string()).unwrap();
        std::fs::write(log_path("rn2-old"), "log2\n").unwrap();
        state
            .store
            .write_async(|conn| {
                conn.execute(
                    "INSERT INTO issues (id, title, session, status, owner_type, created, updated)
                     VALUES ('I-2', 'card2', 'rn2-old', 'todo', 'agent', 1, 1)",
                    [],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .unwrap();
        std::fs::rename(env_path("rn2-old"), env_path("rn2-new")).unwrap(); // the "crash"
        let (st, v) = call(
            &app, "PATCH", "/api/sessions/rn2-old/config", Some(json!({"rename": "rn2-new"})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "retry must converge, got {v}");
        assert_eq!(v["resumed_partial"], json!(true), "{v}");
        assert!(meta_path("rn2-new").exists(), "meta straggler must follow on retry");
        assert!(log_path("rn2-new").exists(), "log straggler must follow on retry");
        {
            let conn = state.store.read().unwrap();
            let sess: String = conn
                .query_row("SELECT session FROM issues WHERE id='I-2'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sess, "rn2-new", "DB straggler must follow on retry");
        }

        // 4. Collision: both envs exist → 409 naming the conflict; and a
        //    rename of a missing session to a missing target stays a 404.
        std::fs::write(env_path("rn3"), "CC_DESC=\"third\"\n").unwrap();
        let (st, v) = call(
            &app, "PATCH", "/api/sessions/rn3/config", Some(json!({"rename": "rn-new"})),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], json!("'rn-new' already exists"));
        let (st, _) = call(
            &app, "PATCH", "/api/sessions/ghost/config", Some(json!({"rename": "also-ghost"})),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

}
