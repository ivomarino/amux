//! Native Chrome profile management + launch (RR-0092; plan lesson L7).
//!
//! Profile locations are the PYTHON server's, not a new scheme — L7's whole
//! lesson is that create-path and use-path must be the same bytes, and the
//! existing logged-in profiles live where `_bu_profile_dir` in amux-server.py
//! resolves them (NOT under `~/.amux/browser-profiles/`, which nothing ever
//! used — verified by grep before writing this):
//! - `default` (or empty)  -> `<amux_home>/playwright-auth/profile`
//! - named, legacy         -> `<amux_home>/playwright-auth/profiles/<name>`
//!   when that directory exists (the 35+ real logged-in profiles are here)
//! - named, otherwise      -> `<chrome-user-data-dir>/<name>` (a Chrome
//!   `--profile-directory` inside the user's real Chrome data dir)
//!
//! NEW profiles are created under `playwright-auth/profiles/<name>` — the
//! amux-owned location — because this launcher passes `--user-data-dir`
//! straight at the profile dir. (Python's create targets the shared Chrome
//! dir for browser-use interop; the native launcher IS the consumer here, so
//! amux-owned is the location whose create-path == use-path.) Existing
//! Chrome-dir profiles still resolve and launch via
//! `--user-data-dir=<chrome dir> --profile-directory=<name>`, exactly like
//! `_bu_profile_launch_target`.
//!
//! Lock-file hygiene: Chrome drops `SingletonLock`/`SingletonCookie`/
//! `SingletonSocket` on a clean exit and leaves them when killed; a stale set
//! blocks the next launch (AMUX-2070). We clean them on demand and at startup
//! reconcile — but ONLY inside amux-owned dirs (`playwright-auth/...`). The
//! real Chrome user-data-dir is the human's live browser; deleting its locks
//! while it runs would corrupt their session, so it is never touched.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// Chrome removes these on clean exit; their presence after exit means the
/// profile was never flushed (Python `_CHROME_SINGLETONS`).
pub const CHROME_SINGLETONS: [&str; 3] = ["SingletonLock", "SingletonCookie", "SingletonSocket"];

pub fn amux_home() -> PathBuf {
    std::env::var("AMUX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"))
                .join(".amux")
        })
}

/// The user's real Chrome user-data-dir (Python `_chrome_user_data_dir`).
pub fn chrome_user_data_dir() -> PathBuf {
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Google/Chrome")
    } else if cfg!(windows) {
        std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("Google/Chrome/User Data")
    } else {
        home.join(".config/google-chrome")
    }
}

/// Python `_bu_profile_dir`: where a profile name lives on disk. Pure in its
/// inputs so tests can drive it hermetically.
pub fn resolve_profile_dir(home: &Path, chrome_dir: &Path, name: &str) -> PathBuf {
    let n = name.trim();
    if n.is_empty() || n == "default" {
        return home.join("playwright-auth").join("profile");
    }
    let legacy = home.join("playwright-auth").join("profiles").join(n);
    if legacy.is_dir() {
        return legacy;
    }
    chrome_dir.join(n)
}

/// How to hand a resolved profile dir to Chrome (Python
/// `_bu_profile_launch_target`): a dir nested in the Chrome user-data-dir is
/// a `--profile-directory`, an amux-owned dir IS the `--user-data-dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    pub user_data_dir: PathBuf,
    pub profile_directory: Option<String>,
}

pub fn launch_target(home: &Path, chrome_dir: &Path, name: &str) -> LaunchTarget {
    let d = resolve_profile_dir(home, chrome_dir, name);
    if d.parent() == Some(chrome_dir) {
        LaunchTarget {
            user_data_dir: chrome_dir.to_path_buf(),
            profile_directory: d.file_name().map(|s| s.to_string_lossy().into_owned()),
        }
    } else {
        LaunchTarget { user_data_dir: d, profile_directory: None }
    }
}

/// Is this dir one amux owns outright (safe to create, clean locks in,
/// delete)? Anything else — notably the real Chrome user-data-dir — is the
/// human's and is never mutated beyond what Chrome itself does.
pub fn is_amux_owned(home: &Path, dir: &Path) -> bool {
    dir.starts_with(home.join("playwright-auth"))
}

// ---- inventory ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BrowserProfile {
    pub name: String,
    pub path: String,
    /// Unix seconds from the profile dir's mtime — Chrome touches the dir on
    /// use, so this is "when was this profile last opened", cheaply.
    pub last_used: Option<i64>,
    /// Only when the caller asked (`?sizes=1`): walking a 565MB profile took
    /// 2.9s in the Python listing and starved the picker, so sizes stay
    /// opt-in here too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mb: Option<f64>,
    /// Registry metadata (`playwright-auth/profiles.json`, shared with the
    /// Python server): which domains this profile is signed into.
    pub domains: Vec<String>,
    pub label: String,
    pub registered: bool,
}

fn dir_mtime_unix(p: &Path) -> Option<i64> {
    let modified = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
}

/// Recursive on-disk size without a walkdir dep (profiles are a few hundred
/// files; the Python listing does the same walk).
pub fn dir_size_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

/// Registry metadata (Python `_bu_registry_load`): name -> {domains, label}.
fn registry_load(home: &Path) -> serde_json::Map<String, serde_json::Value> {
    let path = home.join("playwright-auth").join("profiles.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Inventory of amux-owned profiles: a directory that exists IS a profile
/// (the Python listing's hard-won rule — the registry adds metadata, it is
/// not the source of truth for existence).
pub fn list_profiles(home: &Path, with_sizes: bool) -> Vec<BrowserProfile> {
    let registry = registry_load(home);
    let mut names: std::collections::BTreeSet<String> =
        registry.keys().cloned().collect();
    let profiles_dir = home.join("playwright-auth").join("profiles");
    if let Ok(entries) = std::fs::read_dir(&profiles_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && !name.starts_with('.') {
                names.insert(name);
            }
        }
    }
    if home.join("playwright-auth").join("profile").is_dir() {
        names.insert("default".into());
    }

    let chrome_dir = chrome_user_data_dir();
    names
        .into_iter()
        .map(|name| {
            let dir = resolve_profile_dir(home, &chrome_dir, &name);
            let meta = registry.get(&name).and_then(|v| v.as_object());
            BrowserProfile {
                path: dir.display().to_string(),
                last_used: dir_mtime_unix(&dir),
                // 3-decimal precision: a 2KB profile must not round to 0.0
                // (a nonzero directory reading as empty is a lie; display
                // rounding is the UI's call).
                size_mb: with_sizes
                    .then(|| (dir_size_bytes(&dir) as f64 / (1024.0 * 1024.0) * 1000.0).round() / 1000.0),
                domains: meta
                    .and_then(|m| m.get("domains"))
                    .and_then(|d| d.as_array())
                    .map(|a| {
                        a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
                    })
                    .unwrap_or_default(),
                label: meta
                    .and_then(|m| m.get("label"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .to_string(),
                registered: meta.is_some(),
                name,
            }
        })
        .collect()
}

// ---- lock files -----------------------------------------------------------

/// Which Singleton* entries are present. `symlink_metadata` (lstat), not
/// `exists()`: the entries are symlinks pointing at `<host>-<pid>`, and
/// `exists()` follows the link — a live lock with an unresolvable target
/// would read as absent exactly when the check matters (Python
/// `_chrome_locks_present` carries the same comment).
pub fn locks_present(dir: &Path) -> Vec<String> {
    CHROME_SINGLETONS
        .iter()
        .filter(|n| dir.join(n).symlink_metadata().is_ok())
        .map(|n| n.to_string())
        .collect()
}

/// Remove Singleton* entries from ONE dir. Callers are responsible for only
/// pointing this at amux-owned dirs whose Chrome is known dead.
pub fn clean_locks(dir: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    for n in CHROME_SINGLETONS {
        let p = dir.join(n);
        if p.symlink_metadata().is_ok() && std::fs::remove_file(&p).is_ok() {
            removed.push(n.to_string());
        }
    }
    removed
}

/// Startup reconcile: at boot no amux-launched Chrome exists, so any lock in
/// an amux-owned profile dir is stale by definition and blocks the next
/// launch. Returns (dir, removed-locks) pairs so the caller can log WHAT was
/// cleaned, not just that cleaning happened (ethos rule 4).
pub fn reconcile_locks_at_startup(home: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut dirs = vec![home.join("playwright-auth").join("profile")];
    if let Ok(entries) = std::fs::read_dir(home.join("playwright-auth").join("profiles")) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                dirs.push(e.path());
            }
        }
    }
    let mut cleaned = Vec::new();
    for dir in dirs {
        debug_assert!(is_amux_owned(home, &dir));
        let removed = clean_locks(&dir);
        if !removed.is_empty() {
            cleaned.push((dir, removed));
        }
    }
    cleaned
}

// ---- Chrome launch + CDP over HTTP ----------------------------------------

/// The one browser this server has running (the dashboard's start/stop model
/// is a single session). Starting while one runs stops the old one first.
pub struct RunningBrowser {
    pub profile: String,
    pub user_data_dir: PathBuf,
    pub cdp_port: u16,
    pub started_at: i64,
    child: tokio::process::Child,
}

pub static RUNNING: LazyLock<Mutex<Option<RunningBrowser>>> = LazyLock::new(|| Mutex::new(None));

/// Locate a Chrome/Chromium binary. None is an honest answer the API
/// surfaces as 501 — not a fallback to some other browser.
pub fn chrome_binary() -> Option<PathBuf> {
    let fixed = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    for c in fixed {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"];
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn ephemeral_port() -> anyhow::Result<u16> {
    // Bind :0, read the port, release. A race with another process is
    // possible but Chrome fails loudly if it loses, and CDP wait catches it.
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

#[derive(Debug, Clone, Serialize)]
pub struct StartedBrowser {
    pub profile: String,
    pub pid: Option<u32>,
    pub cdp_port: u16,
    pub cdp_http: String,
    pub user_data_dir: String,
    pub started_at: i64,
}

/// Launch Chrome on a profile. Cleans stale locks first (amux-owned dirs
/// only — see module docs), waits for the CDP HTTP endpoint to answer so a
/// returned port is a WORKING port, and records the child for stop().
pub async fn start(home: &Path, profile: &str, url: &str) -> anyhow::Result<StartedBrowser> {
    let binary = chrome_binary().ok_or_else(|| {
        anyhow::anyhow!("no Chrome/Chromium binary found (looked in /Applications and PATH)")
    })?;

    // One running browser: replace, never silently stack a second Chrome on
    // the same profile (two Chromes on one user-data-dir corrupt it).
    if RUNNING.lock().expect("browser registry poisoned").is_some() {
        let _ = stop(home).await;
    }

    let target = launch_target(home, &chrome_user_data_dir(), profile);
    if is_amux_owned(home, &target.user_data_dir) {
        std::fs::create_dir_all(&target.user_data_dir)?;
        // Our child registry is empty (checked above), so any lock here is a
        // leftover from a dead Chrome and would block this launch.
        let removed = clean_locks(&target.user_data_dir);
        if !removed.is_empty() {
            tracing::info!(dir = %target.user_data_dir.display(), ?removed, "cleaned stale Chrome locks before launch");
        }
    }

    let port = ephemeral_port()?;
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", target.user_data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    if let Some(pd) = &target.profile_directory {
        cmd.arg(format!("--profile-directory={pd}"));
    }
    if !url.trim().is_empty() {
        cmd.arg(url.trim());
    }
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| anyhow::anyhow!("spawn {}: {e}", binary.display()))?;
    let pid = child.id();

    // A port Chrome never bound is not a started browser; wait for CDP HTTP.
    let http = format!("http://127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        match reqwest::Client::new()
            .get(format!("{http}/json/version"))
            .timeout(std::time::Duration::from_secs(1))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => break,
            _ if std::time::Instant::now() > deadline => {
                anyhow::bail!("Chrome spawned (pid {pid:?}) but CDP on port {port} never answered within 12s");
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    }

    let started_at = chrono::Utc::now().timestamp();
    let info = StartedBrowser {
        profile: profile.to_string(),
        pid,
        cdp_port: port,
        cdp_http: http,
        user_data_dir: target.user_data_dir.display().to_string(),
        started_at,
    };
    *RUNNING.lock().expect("browser registry poisoned") = Some(RunningBrowser {
        profile: profile.to_string(),
        user_data_dir: target.user_data_dir,
        cdp_port: port,
        started_at,
        child,
    });
    Ok(info)
}

#[derive(Debug, Clone, Serialize)]
pub struct StopReport {
    pub stopped: bool,
    pub profile: Option<String>,
    /// True when Chrome exited and dropped its own Singleton locks — the
    /// signal that Cookies/Local Storage were flushed (AMUX-2070).
    pub clean_exit: Option<bool>,
    pub locks_cleaned: Vec<String>,
}

/// Stop the tracked Chrome. SIGTERM first — TERM is the signal Chrome
/// flushes Cookies/Local Storage on; SIGKILL is precisely what loses them
/// (Python `_chrome_terminate_automation`'s lesson) — escalating to SIGKILL
/// only if it ignores TERM for 8s.
pub async fn stop(home: &Path) -> StopReport {
    let running = RUNNING.lock().expect("browser registry poisoned").take();
    let Some(mut running) = running else {
        return StopReport { stopped: false, profile: None, clean_exit: None, locks_cleaned: vec![] };
    };

    if let Some(pid) = running.child.id() {
        // std/tokio only offer SIGKILL; /bin/kill sends the TERM we need.
        let _ = tokio::process::Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await;
    }
    let graceful =
        tokio::time::timeout(std::time::Duration::from_secs(8), running.child.wait()).await;
    if graceful.is_err() {
        let _ = running.child.kill().await; // last resort; may lose storage flush
        let _ = running.child.wait().await;
    }

    let clean_exit = locks_present(&running.user_data_dir).is_empty();
    let locks_cleaned = if !clean_exit && is_amux_owned(home, &running.user_data_dir) {
        // The child is reaped; remaining locks are stale by definition.
        clean_locks(&running.user_data_dir)
    } else {
        vec![]
    };
    StopReport {
        stopped: true,
        profile: Some(running.profile),
        clean_exit: Some(clean_exit),
        locks_cleaned,
    }
}

/// CDP over plain HTTP: the tab list. (The WS endpoints are out of scope
/// until the websocket dep decision — see api/browser.rs screenshot.)
pub async fn cdp_list(port: u16) -> anyhow::Result<serde_json::Value> {
    let r = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    Ok(r.json().await?)
}

/// CDP over plain HTTP: open a new tab. Chrome 111+ requires PUT on
/// /json/new; older builds only accept GET — try PUT, fall back.
pub async fn cdp_new_tab(port: u16, url: &str) -> anyhow::Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let endpoint = format!(
        "http://127.0.0.1:{port}/json/new?{}",
        serde_urlencoded_url(url)
    );
    let put = client
        .put(&endpoint)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    let resp = match put {
        Ok(r) if r.status().is_success() => r,
        _ => client
            .get(&endpoint)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?,
    };
    Ok(resp.json().await?)
}

/// Minimal query-encoding for the one place we build a query string by hand
/// (reqwest's `query()` would double-encode an already-composed URL value).
fn serde_urlencoded_url(url: &str) -> String {
    let mut out = String::from("url=");
    for b in url.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---- tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // All tests run against TEMP dirs only — never the live Chrome profiles
    // (repo rule: tests must not touch real profile contents), and never
    // launch Chrome except the #[ignore]d gated test at the bottom.

    fn fake_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn profile_resolution_matches_python_precedence() {
        let home = fake_home();
        let chrome = home.path().join("fake-chrome-udd");
        std::fs::create_dir_all(&chrome).unwrap();

        // default -> the bare playwright-auth/profile dir
        assert_eq!(
            resolve_profile_dir(home.path(), &chrome, "default"),
            home.path().join("playwright-auth/profile")
        );
        assert_eq!(
            resolve_profile_dir(home.path(), &chrome, "  "),
            home.path().join("playwright-auth/profile")
        );

        // a legacy dir that exists wins over the chrome location
        let legacy = home.path().join("playwright-auth/profiles/gh");
        std::fs::create_dir_all(&legacy).unwrap();
        assert_eq!(resolve_profile_dir(home.path(), &chrome, "gh"), legacy);

        // unknown names resolve into the chrome user-data-dir
        assert_eq!(resolve_profile_dir(home.path(), &chrome, "brandnew"), chrome.join("brandnew"));
    }

    #[test]
    fn launch_target_splits_chrome_dir_profiles() {
        let home = fake_home();
        let chrome = home.path().join("fake-chrome-udd");
        std::fs::create_dir_all(&chrome).unwrap();

        // amux-owned dir IS the user-data-dir
        let legacy = home.path().join("playwright-auth/profiles/gh");
        std::fs::create_dir_all(&legacy).unwrap();
        let t = launch_target(home.path(), &chrome, "gh");
        assert_eq!(t.user_data_dir, legacy);
        assert_eq!(t.profile_directory, None);

        // chrome-dir profile becomes --profile-directory under the parent
        let t = launch_target(home.path(), &chrome, "Work");
        assert_eq!(t.user_data_dir, chrome);
        assert_eq!(t.profile_directory.as_deref(), Some("Work"));
    }

    #[test]
    fn inventory_lists_dirs_and_registry_metadata() {
        let home = fake_home();
        let profiles = home.path().join("playwright-auth/profiles");
        std::fs::create_dir_all(profiles.join("github")).unwrap();
        std::fs::write(profiles.join("github/Cookies"), vec![0u8; 2048]).unwrap();
        std::fs::create_dir_all(home.path().join("playwright-auth/profile")).unwrap();
        std::fs::write(
            home.path().join("playwright-auth/profiles.json"),
            r#"{"github":{"domains":["github.com"],"label":"GH"}}"#,
        )
        .unwrap();

        let list = list_profiles(home.path(), true);
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"github"), "dir-backed profile listed: {names:?}");
        assert!(names.contains(&"default"), "default profile listed: {names:?}");

        let gh = list.iter().find(|p| p.name == "github").unwrap();
        assert!(gh.registered);
        assert_eq!(gh.domains, vec!["github.com"]);
        assert_eq!(gh.label, "GH");
        assert!(gh.last_used.is_some(), "mtime-derived last_used");
        assert!(gh.size_mb.unwrap() > 0.0, "walked size includes the 2KB cookie file");

        // sizes are opt-in
        let slim = list_profiles(home.path(), false);
        assert!(slim.iter().all(|p| p.size_mb.is_none()));
    }

    #[test]
    fn lock_cleanup_removes_singletons_including_dangling_symlinks() {
        let home = fake_home();
        let prof = home.path().join("playwright-auth/profiles/x");
        std::fs::create_dir_all(&prof).unwrap();
        // Real Chrome locks are symlinks at "<host>-<pid>" whose target does
        // not exist — the exact case exists() gets wrong and lstat gets right.
        #[cfg(unix)]
        std::os::unix::fs::symlink("nohost-99999", prof.join("SingletonLock")).unwrap();
        #[cfg(not(unix))]
        std::fs::write(prof.join("SingletonLock"), "x").unwrap();
        std::fs::write(prof.join("SingletonCookie"), "c").unwrap();
        std::fs::write(prof.join("Preferences"), "{}").unwrap();

        let present = locks_present(&prof);
        assert!(present.contains(&"SingletonLock".to_string()), "lstat sees the dangling symlink");
        assert!(present.contains(&"SingletonCookie".to_string()));

        let cleaned = reconcile_locks_at_startup(home.path());
        assert_eq!(cleaned.len(), 1);
        let (dir, removed) = &cleaned[0];
        assert_eq!(dir, &prof);
        assert!(removed.contains(&"SingletonLock".to_string()));
        assert!(removed.contains(&"SingletonCookie".to_string()));
        assert!(locks_present(&prof).is_empty(), "locks actually gone");
        assert!(prof.join("Preferences").exists(), "profile CONTENTS untouched");
    }

    #[test]
    fn amux_owned_boundary() {
        let home = fake_home();
        assert!(is_amux_owned(home.path(), &home.path().join("playwright-auth/profiles/x")));
        assert!(is_amux_owned(home.path(), &home.path().join("playwright-auth/profile")));
        assert!(!is_amux_owned(home.path(), &chrome_user_data_dir()));
        assert!(!is_amux_owned(home.path(), Path::new("/tmp/elsewhere")));
    }

    /// Live-Chrome smoke test: launches a real Chrome on a THROWAWAY temp
    /// user-data-dir (never a saved profile), checks CDP answers, stops it.
    /// #[ignore]d: `cargo test -- --ignored` runs it only where a human asked.
    #[tokio::test]
    #[ignore = "launches a real Chrome; run explicitly with -- --ignored"]
    async fn live_chrome_start_stop() {
        if chrome_binary().is_none() {
            eprintln!("no Chrome binary on this machine; skipping");
            return;
        }
        let home = fake_home();
        // Use a scratch profile name so the target dir is amux-owned + temp.
        let info = start(home.path(), "default", "about:blank").await.expect("start");
        assert!(info.cdp_port > 0);
        let tabs = cdp_list(info.cdp_port).await.expect("cdp /json/list");
        assert!(tabs.is_array());
        let report = stop(home.path()).await;
        assert!(report.stopped);
    }
}
