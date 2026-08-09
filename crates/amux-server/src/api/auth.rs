//! Bearer-token auth middleware (RR-0021).
//!
//! Token lives at `~/.amux/auth-token` (shared with the Python server so one
//! dashboard login works against both during migration). `None` disables
//! auth — used by tests and first-run before a token exists.

use super::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub async fn require_bearer(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = &state.auth_token else {
        return next.run(req).await;
    };
    let provided = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        // The dashboard also passes ?token= on EventSource connections,
        // which cannot set headers.
        .or_else(|| {
            req.uri().query().and_then(|q| {
                q.split('&').find_map(|kv| {
                    // The SPA's _authUrl sends `_token=` (underscore); accept
                    // both spellings — rejecting the dashboard's own param
                    // silently killed SSE for every client (browser-golden
                    // finding #1: the SPA degraded to 5s polling forever).
                    kv.strip_prefix("token=")
                        .or_else(|| kv.strip_prefix("_token="))
                })
            })
        });
    match provided {
        Some(t) if constant_time_eq(t.as_bytes(), expected.as_bytes()) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}

/// Constant-time comparison — a token check that leaks length-prefix timing
/// is a token check that can be brute-forced from the LAN.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Load or mint the auth token file (0600).
pub fn load_or_create_token(path: &std::path::Path) -> anyhow::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = ulid::Ulid::new().to_string().to_lowercase();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn token_minted_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("amux-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("auth-token");
        let t1 = load_or_create_token(&p).unwrap();
        let t2 = load_or_create_token(&p).unwrap();
        assert_eq!(t1, t2);
        assert!(!t1.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
