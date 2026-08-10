//! Legacy-port accounting — turning "can we drop the 8822 bind yet?" into a
//! number instead of a guess.
//!
//! # Why this exists
//!
//! The server answers a second, historical port (`AMUX_RS_LEGACY_PORT`, 8822
//! here) because ~60 live fleet sessions carry `AMUX_URL=https://localhost:8822`
//! in their PROCESS env, and a live process's env cannot be rotated. Every one
//! of them keeps working only because that bind exists. The address itself is
//! retired: every config, doc, CLI default and freshly-spawned session now says
//! 8824.
//!
//! So the bind is pure carry-over for processes that predate the cutover, and
//! it should be dropped — but "is anything still calling it?" was answerable
//! only by guessing at how many old sessions were left. A retirement decision
//! made from a guess is how you either keep a legacy port forever or break the
//! fleet at 2am. Ethos rule 4: if a wrong answer would not be detectable from
//! the data we keep, the missing instrument IS the bug.
//!
//! # THE EXIT CONDITION for deleting the legacy bind
//!
//! Drop `AMUX_RS_LEGACY_PORT` from `~/.amux/server.env` (and this module, and
//! the bind block in `lib.rs`) when BOTH hold:
//!
//!   1. `GET /api/debug/legacy-port` reports `hits_total: 0` across a window of
//!      at least **7 days of continuous uptime** (`window.hours_elapsed >= 168`
//!      with `hits_total == 0`). Seven days because a session can sit parked for
//!      days and then take one turn; an hour of silence proves nothing.
//!   2. `clients` is empty — nothing has been seen at all, as opposed to "a
//!      known caller went quiet". Any entry there names a specific IP and
//!      user-agent you can go and fix.
//!
//! Until then the hourly report is the ticker: a WARN line while anything is
//! still arriving, an INFO line saying zero once nothing is. `grep 'legacy
//! port' ~/.amux/logs/server-rs.log | grep WARN` returning nothing for a week
//! is the same signal as (1), readable without the API.
//!
//! # How the count is taken
//!
//! By LAYER on the legacy listener's router clone, not by sniffing the `Host`
//! header. The two listeners share one `app`, so the legacy one gets its own
//! `.layer()` and a request is counted iff it physically arrived on that
//! socket. A `Host` header is client-supplied and would count a request that
//! merely *said* 8822 while arriving on 8824 — which, on the one measurement
//! the retirement decision rests on, is the difference between deleting the
//! bind and breaking 60 sessions.

use axum::extract::{ConnectInfo, Request};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Distinct (ip, user-agent) pairs retained. A cap is needed so a scanner
/// cannot grow this without bound, and the number of pairs it DROPPED is
/// reported alongside — an evidence cap that hides the fact it capped is how
/// you conclude "only these three callers" from a truncated list.
const MAX_CLIENTS: usize = 200;

/// One distinct caller of the legacy port.
#[derive(Clone, Debug, Default)]
pub struct ClientTally {
    pub count: u64,
    pub first_seen: u64,
    pub last_seen: u64,
    pub last_path: String,
}

#[derive(Debug, Default)]
struct Inner {
    /// The port being answered. 0 = the bind is not enabled at all.
    port: u16,
    /// Process start (== window start; the count is per-uptime, and the
    /// debug surface says so rather than implying an all-time total).
    started_at: u64,
    hits_total: u64,
    last_hit: u64,
    /// Reset each hourly report, so the log line reads "in the last hour".
    hour_hits: u64,
    hour_started: u64,
    clients: BTreeMap<String, ClientTally>,
    clients_dropped: u64,
}

static STATE: OnceLock<Mutex<Inner>> = OnceLock::new();

fn state() -> &'static Mutex<Inner> {
    STATE.get_or_init(|| Mutex::new(Inner::default()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Record that the bind came up, so the debug surface can distinguish
/// "enabled, zero traffic" (the state we are waiting for) from "not enabled"
/// (nothing to decide). Without this they both render as zero hits.
pub fn arm(port: u16) {
    if let Ok(mut s) = state().lock() {
        let t = now();
        s.port = port;
        s.started_at = t;
        s.hour_started = t;
    }
}

/// Middleware for the LEGACY listener only. Counts the request, tags who sent
/// it, and passes it through unchanged — the legacy port must stay
/// byte-identical to the primary one, since sessions are depending on it.
pub async fn count(req: Request, next: Next) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(a)| a.ip().to_string())
        .unwrap_or_else(|| "unknown".into());
    let ua = req
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .chars()
        .take(120)
        .collect::<String>();
    let path = req.uri().path().to_string();

    if let Ok(mut s) = state().lock() {
        let t = now();
        s.hits_total += 1;
        s.hour_hits += 1;
        s.last_hit = t;
        let key = format!("{ip} {ua}");
        if let Some(e) = s.clients.get_mut(&key) {
            e.count += 1;
            e.last_seen = t;
            e.last_path = path;
        } else if s.clients.len() < MAX_CLIENTS {
            s.clients.insert(
                key,
                ClientTally { count: 1, first_seen: t, last_seen: t, last_path: path },
            );
        } else {
            s.clients_dropped += 1;
        }
    }
    next.run(req).await
}

/// Snapshot for the debug surface / the reporter, as JSON.
pub fn snapshot() -> serde_json::Value {
    let Ok(s) = state().lock() else {
        return serde_json::json!({"error": "legacy-port state poisoned"});
    };
    let t = now();
    let mut clients: Vec<_> = s
        .clients
        .iter()
        .map(|(k, v)| {
            let (ip, ua) = k.split_once(' ').unwrap_or((k.as_str(), "-"));
            serde_json::json!({
                "ip": ip,
                "user_agent": ua,
                "count": v.count,
                "first_seen": v.first_seen,
                "last_seen": v.last_seen,
                "last_path": v.last_path,
            })
        })
        .collect();
    // Loudest caller first: the list is read to decide who to go and fix.
    clients.sort_by_key(|c| std::cmp::Reverse(c["count"].as_u64().unwrap_or(0)));
    let elapsed = t.saturating_sub(s.started_at);
    serde_json::json!({
        "enabled": s.port != 0,
        "port": s.port,
        "canonical_port": std::env::var("AMUX_RS_PORT").unwrap_or_else(|_| crate::config::DEFAULT_PORT.to_string()),
        // Per-UPTIME, not all-time: the server re-execs on every install, and
        // reporting a restarted counter as a lifetime total would make the
        // retirement window look satisfied when it had just been reset.
        "hits_total": s.hits_total,
        "window": {
            "started_at": s.started_at,
            "hours_elapsed": elapsed as f64 / 3600.0,
            "resets_on_restart": true,
        },
        "last_hit": if s.last_hit == 0 { serde_json::Value::Null } else { s.last_hit.into() },
        "clients": clients,
        "clients_dropped": s.clients_dropped,
        "retire_when": "hits_total == 0 with window.hours_elapsed >= 168 (7d) AND clients empty \
                        -> drop AMUX_RS_LEGACY_PORT from ~/.amux/server.env and delete the bind \
                        block in lib.rs. See docs/rust-migration/server-boundary.md.",
        "ready_to_retire": s.hits_total == 0 && s.clients.is_empty() && elapsed >= 7 * 24 * 3600,
    })
}

/// `GET /api/debug/legacy-port`.
pub async fn debug() -> axum::Json<serde_json::Value> {
    axum::Json(snapshot())
}

/// Hourly ticker. WARN while the legacy port is still being used (with who is
/// using it, so the line is actionable on its own), INFO when it is not —
/// because ZERO is the signal the retirement is waiting for, and a reporter
/// that goes silent on zero cannot be distinguished from a reporter that died.
pub async fn run_reporter() {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    tick.tick().await; // fires immediately; skip the t=0 tick
    loop {
        tick.tick().await;
        let (port, hour_hits, total, top, dropped, elapsed_h) = {
            let Ok(mut s) = state().lock() else { continue };
            if s.port == 0 {
                continue; // bind not enabled — nothing to report
            }
            let mut top: Vec<(String, u64)> =
                s.clients.iter().map(|(k, v)| (k.clone(), v.count)).collect();
            top.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
            top.truncate(5);
            let hour_hits = s.hour_hits;
            s.hour_hits = 0;
            s.hour_started = now();
            let elapsed_h = now().saturating_sub(s.started_at) as f64 / 3600.0;
            (s.port, hour_hits, s.hits_total, top, s.clients_dropped, elapsed_h)
        };
        if hour_hits == 0 {
            tracing::info!(
                port,
                hits_last_hour = 0,
                hits_this_uptime = total,
                uptime_hours = format!("{elapsed_h:.1}"),
                "legacy port idle — retire when this stays 0 for 7d (GET /api/debug/legacy-port)"
            );
        } else {
            let callers = top
                .iter()
                .map(|(k, c)| format!("{k} x{c}"))
                .collect::<Vec<_>>()
                .join("; ");
            tracing::warn!(
                port,
                hits_last_hour = hour_hits,
                hits_this_uptime = total,
                uptime_hours = format!("{elapsed_h:.1}"),
                clients_dropped = dropped,
                %callers,
                "legacy port STILL IN USE — bind cannot be dropped yet"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bind being enabled with zero traffic is the state the retirement is
    /// waiting for; it must not render the same as the bind being off, or the
    /// number the owner reads cannot distinguish "safe to drop" from "already
    /// dropped, nothing measured".
    #[test]
    fn armed_and_idle_is_distinguishable_from_disabled() {
        // Fresh state (default) = never armed.
        let off = snapshot();
        assert_eq!(off["enabled"], false, "unarmed must report enabled:false");
        assert_eq!(off["port"], 0);

        arm(8822);
        let on = snapshot();
        assert_eq!(on["enabled"], true, "armed must report enabled:true");
        assert_eq!(on["port"], 8822);
        assert_eq!(on["hits_total"], 0, "arming must not fabricate a hit");
        // Just-armed is NOT ready to retire: the 7d window has not elapsed. A
        // check that says "ready" the instant the server boots is the exact
        // false-green this instrument exists to prevent.
        assert_eq!(
            on["ready_to_retire"], false,
            "a fresh window must never read as ready to retire"
        );
    }
}
