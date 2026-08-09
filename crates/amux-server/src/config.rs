//! Server configuration: `~/.amux/server.env` + environment + defaults
//! (RR-0020, Invariant 2).
//!
//! Precedence (highest wins): process environment > server.env > defaults.
//! This mirrors the Python server's `os.environ.setdefault` semantics so the
//! same server.env file drives both servers during the strangler-fig
//! migration. The four-tier Org/Global/Group/Worker resolution for
//! worker-scoped config happens in amux-core's scope module against DB rows;
//! this file only handles PROCESS-level configuration.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The Rust server binds 8823 by default while the Python server owns 8822.
/// Phase 11's cutover flips this; until then both run side by side against
/// the same DB (WAL allows concurrent readers).
pub const DEFAULT_PORT: u16 = 8823;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `~/.amux` — sessions dir, DB, TLS material, tokens.
    pub amux_home: PathBuf,
    pub port: u16,
    /// Path to the SQLite database (shared with the Python server).
    pub db_path: PathBuf,
    /// Everything from server.env plus process env overlays, for
    /// worker-environment assembly later.
    pub env: BTreeMap<String, String>,
}

impl ServerConfig {
    /// Load configuration. Pure given its inputs — callers pass the home dir
    /// and process env so tests can drive it hermetically.
    pub fn load(home: PathBuf, process_env: &BTreeMap<String, String>) -> Self {
        let mut env = parse_env_file(&home.join("server.env"));
        // PYTHON-PARITY SETDEFAULT, for real: export server.env values into
        // the PROCESS env when the process doesn't already set them. The doc
        // above always claimed setdefault semantics, but values only reached
        // the Config struct — every `std::env::var()` read site (the
        // AMUX_RS_SCHEDULER gate, AMUX_HERDR_SESSION, the caps/knobs) saw
        // nothing, so server.env flags silently didn't work (live incident
        // 2026-08-09: scheduler stayed in shadow mode with the flag set).
        for (k, v) in env.iter() {
            if !process_env.contains_key(k) && std::env::var_os(k).is_none() {
                std::env::set_var(k, v);
            }
        }
        // Process env wins over server.env (same rule as Python's setdefault).
        for (k, v) in process_env {
            env.insert(k.clone(), v.clone());
        }
        let port = env
            .get("AMUX_RS_PORT")
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        let db_path = env
            .get("AMUX_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("amux.db"));
        ServerConfig {
            amux_home: home,
            port,
            db_path,
            env,
        }
    }

    pub fn from_process_env() -> Self {
        let home = std::env::var("AMUX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_home().join(".amux")
            });
        let process_env: BTreeMap<String, String> = std::env::vars().collect();
        Self::load(home, &process_env)
    }

    pub fn tls_dir(&self) -> PathBuf {
        self.amux_home.join("tls")
    }

    /// Python's `_AUTH_TOKEN_FILE` (amux-server.py:700): `auth_token`,
    /// UNDERSCORE. This crate shipped reading `auth-token` (dash) and minted
    /// its own token there, so every client holding the real shared token got
    /// 401s from this server while the auth docstring claimed the file was
    /// shared. The stale dash-file may still exist on machines that ran the
    /// old build; nothing reads it anymore.
    pub fn auth_token_path(&self) -> PathBuf {
        self.amux_home.join("auth_token")
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"))
}

/// Parse a KEY=VALUE env file. Supports `#` comments, blank lines, single or
/// double quoted values, and `export ` prefixes — the shapes that appear in
/// real server.env files today.
pub fn parse_env_file(path: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let mut v = v.trim();
        if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
            || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
        {
            v = &v[1..v.len() - 1];
        }
        if !k.is_empty() {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_file_shapes() {
        let dir = std::env::temp_dir().join(format!("amux-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("server.env");
        std::fs::write(
            &p,
            "# comment\nAMUX_S3_BUCKET=my-bucket\nQUOTED=\"has spaces\"\nexport EXPORTED='single'\n\nBROKEN_LINE\n",
        )
        .unwrap();
        let env = parse_env_file(&p);
        assert_eq!(env.get("AMUX_S3_BUCKET").unwrap(), "my-bucket");
        assert_eq!(env.get("QUOTED").unwrap(), "has spaces");
        assert_eq!(env.get("EXPORTED").unwrap(), "single");
        assert!(!env.contains_key("BROKEN_LINE"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn process_env_beats_server_env() {
        let dir = std::env::temp_dir().join(format!("amux-cfg-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("server.env"), "AMUX_RS_PORT=9000\nA=file\n").unwrap();
        let mut penv = BTreeMap::new();
        penv.insert("AMUX_RS_PORT".to_string(), "9001".to_string());
        let cfg = ServerConfig::load(dir.clone(), &penv);
        assert_eq!(cfg.port, 9001);
        assert_eq!(cfg.env.get("A").unwrap(), "file");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn defaults_when_nothing_set() {
        let dir = std::env::temp_dir().join(format!("amux-cfg-test3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = ServerConfig::load(dir.clone(), &BTreeMap::new());
        assert_eq!(cfg.port, DEFAULT_PORT);
        assert_eq!(cfg.db_path, dir.join("amux.db"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
