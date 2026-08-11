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

/// Distinct paths remembered per caller. `last_path` alone cannot tell two
/// callers wearing the same user-agent apart, and on this port they were:
/// `curl/8.7.1` is BOTH the Claude Code report hooks and every agent's own
/// `curl $AMUX_URL/api/...`, while `Python-urllib` is BOTH the git pre-commit
/// guard and the PreToolUse Bash guard. Draining the port means fixing callers
/// one at a time and watching the number fall, and a tally that collapses four
/// callers into two rows cannot show which fix worked (ethos rule 4 — the
/// instrument must be able to express the discriminator). Capped, with the
/// drop count reported, for the same reason [`MAX_CLIENTS`] is.
const MAX_PATHS_PER_CLIENT: usize = 24;

/// One distinct caller of the legacy port.
#[derive(Clone, Debug, Default)]
pub struct ClientTally {
    pub count: u64,
    pub first_seen: u64,
    pub last_seen: u64,
    pub last_path: String,
    /// path -> hits. See [`MAX_PATHS_PER_CLIENT`].
    pub paths: BTreeMap<String, u64>,
    pub paths_dropped: u64,
}

/// Marker inserted into the request extensions by [`count`], so a handler can
/// tell that THIS request arrived on the retired socket.
///
/// The dashboard shell is the one response that has to care: a client that
/// loaded the SPA from the legacy origin keeps every relative `/api/...` fetch
/// on that origin forever, so the shell is the only place a browser-side
/// migration can be offered. Everything else must stay byte-identical between
/// the two listeners — sessions are depending on it.
#[derive(Clone, Copy, Debug)]
pub struct OnLegacyListener(pub u16);

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

/// Where the observation window survives a restart (AMUX-2769).
///
/// The window used to be per-UPTIME, and the reason was sound: a counter that
/// resets and then reports itself as a lifetime total "would make the
/// retirement window look satisfied when it had just been reset". Safe — but
/// it made the exit UNREACHABLE, which is the failure this file is supposed to
/// prevent, one level up. `ready_to_retire` required `hours_elapsed >= 168`
/// (7d) on a process that re-execs every time anyone commits to `crates/`:
/// measured 2026-08-11, the builder had installed 316 times and the window
/// stood at 0.71 hours. The countdown reset before it could ever finish, so
/// the bind that CLAUDE.md calls "a countdown, not an address" had no way to
/// reach zero.
///
/// Persisting `last_hit` fixes it without giving up the safety, because the
/// verdict now keys on QUIET TIME (`now - last_hit`) rather than on process
/// uptime. That is also what the prose always said — "zero hits for 7 days" —
/// and unlike an uptime window it cannot be satisfied by a restart: a restart
/// does not make traffic older.
fn persist_path() -> std::path::PathBuf {
    // Overridable so the lifecycle test below drives the SHIPPED load/save path
    // against a temp file instead of this machine's real window. Without it the
    // test reads whatever traffic the running fleet has recorded and
    // "arming must not fabricate a hit" fails with hits_total=1 — a test whose
    // verdict depends on the host, which is the same trap as an env-mutating
    // test racing its siblings (see this module's own test doc).
    if let Ok(p) = std::env::var("AMUX_LEGACY_PORT_STATE") {
        if !p.trim().is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    crate::config::amux_home().join("legacy-port-window.json")
}

/// Load the persisted window. Absent/corrupt file = a fresh window, which is
/// the conservative direction: it delays retirement, never advances it.
fn load_persisted() -> (u64, u64, u64) {
    let Ok(txt) = std::fs::read_to_string(persist_path()) else { return (0, 0, 0) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { return (0, 0, 0) };
    (
        v.get("first_armed").and_then(|x| x.as_u64()).unwrap_or(0),
        v.get("hits_total").and_then(|x| x.as_u64()).unwrap_or(0),
        v.get("last_hit").and_then(|x| x.as_u64()).unwrap_or(0),
    )
}

fn save_persisted(first_armed: u64, hits_total: u64, last_hit: u64) {
    let body = serde_json::json!({
        "first_armed": first_armed,
        "hits_total": hits_total,
        "last_hit": last_hit,
        "note": "legacy 8822 observation window; survives restarts so the 7-day \
                 quiet period can actually accumulate (AMUX-2769)",
    });
    let p = persist_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, body.to_string());
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
        let (first_armed, hits, last) = load_persisted();
        s.port = port;
        // The window CONTINUES across restarts; only a never-armed machine
        // starts one. hits_total and last_hit are restored with it, so the
        // quiet period is measured from real traffic rather than from boot.
        s.started_at = if first_armed > 0 { first_armed } else { t };
        s.hits_total = hits;
        s.last_hit = last;
        s.hour_started = t;
        // The PER-UPTIME fields start empty, because that is what arming means:
        // this process has begun answering. In production arm() runs once on a
        // fresh process so these are already default — stating it makes the
        // split explicit (persisted: window + traffic recency; per-uptime:
        // who is calling right now) and lets the test model a restart honestly
        // instead of inheriting the previous phase's clients.
        s.clients.clear();
        s.clients_dropped = 0;
        s.hour_hits = 0;
        save_persisted(s.started_at, s.hits_total, s.last_hit);
    }
}

/// Middleware for the LEGACY listener only. Counts the request, tags who sent
/// it, and passes it through unchanged — the legacy port must stay
/// byte-identical to the primary one, since sessions are depending on it.
pub async fn count(mut req: Request, next: Next) -> Response {
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
    // The discriminator this counter was missing (AMUX-2769). ip+ua collapses
    // the WHOLE fleet into one row — every lane is 127.0.0.1 wearing curl/8 —
    // and the exit decision needs to separate two populations that look
    // identical there:
    //   * a pre-cutover SESSION whose process env still says 8822. Cannot be
    //     rotated in a live process, harmless, drains when that lane restarts.
    //   * an UNATTRIBUTED caller — new code, a doc, a script someone just
    //     wrote against the retired address. That is a real defect to fix.
    // Waiting cannot retire the port while the first group is running, and
    // until now nothing could tell the reader which group the traffic was.
    let session = crate::api::groups::hdr_worker(req.headers())
        .trim()
        .chars()
        .take(64)
        .collect::<String>();
    let path = req.uri().path().to_string();

    let port = record(&ip, &ua, &session, &path);
    // Tell the SHELL handler it is answering on the retired socket. Only the
    // dashboard document acts on this; the API surface stays identical. Set
    // outside the lock so a poisoned counter cannot silently disable the
    // migration — the two failures are unrelated and must not be coupled.
    req.extensions_mut()
        .insert(OnLegacyListener(if port == 0 { crate::config::DEFAULT_PORT } else { port }));
    next.run(req).await
}

/// The whole tally, extracted from [`count`] so the test drives THIS and not a
/// hand-written imitation of it. The previous test poked `state()` directly and
/// so could not have caught a bug in the accounting itself — it asserted about
/// numbers it had written by hand (ethos rule 7: test the shipped code path).
///
/// Returns the armed legacy port, or 0 when the bind is not enabled.
fn record(ip: &str, ua: &str, session: &str, path: &str) -> u16 {
    let Ok(mut s) = state().lock() else { return 0 };
    let t = now();
    s.hits_total += 1;
    s.hour_hits += 1;
    let prev_last = s.last_hit;
    s.last_hit = t;
    let port = s.port;
    // THROTTLED to once a minute. `last_hit` is what the retirement verdict
    // reads, and it only needs to be accurate to well within the 7-day quiet
    // period — writing a file on every request would put a syscall in the path
    // of a listener whose entire purpose is to be byte-identical to the
    // primary one. A crash can lose at most 60s of recency, which moves the
    // verdict in the CONSERVATIVE direction (an older last_hit would only ever
    // make retirement look closer, and we round the other way by persisting
    // the first hit in a window immediately).
    let should_persist = prev_last == 0 || t.saturating_sub(prev_last) >= 60;
    let persist_args = (s.started_at, s.hits_total, s.last_hit);
    // TAB-separated: a user-agent contains spaces, so the old
    // `"{ip} {ua}"` + `split_once(' ')` only worked because ip has none.
    // A third field needs an unambiguous separator.
    let key = format!("{ip}\t{ua}\t{session}");
    // Decide admission BEFORE taking the entry: `get_mut(..) else entry(..)`
    // borrows `s.clients` across both arms and does not compile.
    let admit = s.clients.contains_key(&key) || s.clients.len() < MAX_CLIENTS;
    if !admit {
        s.clients_dropped += 1;
        drop(s);
        if should_persist {
            save_persisted(persist_args.0, persist_args.1, persist_args.2);
        }
        return port;
    }
    let e = s.clients.entry(key).or_insert(ClientTally { first_seen: t, ..Default::default() });
    e.count += 1;
    e.last_seen = t;
    e.last_path = path.to_string();
    if let Some(c) = e.paths.get_mut(path) {
        *c += 1;
    } else if e.paths.len() < MAX_PATHS_PER_CLIENT {
        e.paths.insert(path.to_string(), 1);
    } else {
        e.paths_dropped += 1;
    }
    // PERSIST LAST, never with an early return. The first version returned
    // straight after saving and so skipped the client tally below — the hour
    // WARN line lost its caller and the module's own lifecycle test caught it
    // ("the WARN line must be able to name the caller"). The save is a side
    // effect of recording a hit; it must not decide whether the hit is
    // recorded.
    drop(s);
    if should_persist {
        save_persisted(persist_args.0, persist_args.1, persist_args.2);
    }
    port
}

/// The canonical port this server answers on — the one every client should be
/// using. Read from the same place [`crate::config`] reads it, so there is one
/// definition of "where amux is" and not a second literal to keep in step.
pub fn canonical_port() -> u16 {
    std::env::var("AMUX_RS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(crate::config::DEFAULT_PORT)
}

/// Ports this server used to answer and no longer does. Consumed by
/// `endpoint.json` so a hook inheriting a stale `AMUX_URL` can still self-heal
/// after the bind is gone (AMUX-2946).
const RETIRED_PORTS: &[u16] = &[8822];

/// Publish where this server actually is, into `<amux_home>/endpoint.json`.
///
/// # Why a file, and why the server writes it
///
/// The legacy bind exists because ~55 live `claude` processes carry
/// `AMUX_URL=https://localhost:8822` in their PROCESS env, which cannot be
/// rotated without killing them. Every hook those processes spawn — the report
/// hooks, the PreToolUse git guard, the pre-commit staged-guard — inherits that
/// stale value, so "fix the hook's DEFAULT" (`${AMUX_URL:-…8824}`) changes
/// nothing at all: the default only fires when the variable is UNSET, and it
/// never is. That fix was shipped and was inert for all 55 (ethos rule 1 — the
/// capability existed and reached nobody).
///
/// A one-shot hook has exactly one way to notice its inherited URL is stale:
/// ask something on the machine that knows better. This file is that. The
/// SERVER writes it because the server is the only party that knows both ports
/// without guessing, and because it must stay true in the cloud image too,
/// where the numbers are reversed (`AMUX_RS_PORT=8822` there) — a hook with a
/// hardcoded `8822 -> 8824` rewrite would break that deployment, while a hook
/// that reads this file is correct in both. Single codebase, no build flags.
///
/// The consumer rule is deliberately narrow, and is documented here because
/// this file is its contract: **if your URL is `localhost:<legacy_port>`, use
/// `canonical_url` instead.** Not "always prefer canonical" — a session
/// pointing at a dev instance on another port, or at a remote amux, is making a
/// deliberate choice and must be left alone.
pub fn publish_endpoint(amux_home: &std::path::Path, canonical: u16, legacy: Option<u16>) {
    let body = serde_json::json!({
        "canonical_url": format!("https://localhost:{canonical}"),
        "canonical_port": canonical,
        "legacy_port": legacy,
        // RETIRED PORTS OUTLIVE THE BIND, and that is the whole point.
        //
        // The self-heal in .git/hooks/amux-staged-guard keyed on `legacy_port`
        // being non-null: "if your AMUX_URL is the legacy port, use
        // canonical_url". The moment the bind was dropped this began
        // publishing null, the condition went false, and the hook fell back to
        // the stale inherited URL — so the mechanism built for exactly this
        // moment stopped working at exactly this moment. Measured immediately:
        // "staged-guard: NOT ENFORCED — connection refused", i.e. cross-session
        // sweep protection off for every pre-cutover session at once.
        //
        // A retired address needs to stay KNOWN after it stops being served,
        // because the processes that still name it are precisely the ones that
        // cannot be told anything else.
        "retired_ports": RETIRED_PORTS,
        "pid": std::process::id(),
        "written_at": now(),
        "consumer_rule": "if your AMUX_URL is https://localhost:<legacy_port>, use canonical_url",
    });
    let path = amux_home.join("endpoint.json");
    // Write-then-rename: a hook reading this file mid-write must never see a
    // truncated JSON document and fall back to the stale env, which is the
    // exact failure it is here to prevent.
    let tmp = amux_home.join("endpoint.json.tmp");
    let ok = std::fs::write(&tmp, format!("{body}\n"))
        .and_then(|_| std::fs::rename(&tmp, &path))
        .is_ok();
    if ok {
        tracing::info!(?path, canonical, ?legacy, "published endpoint.json (stale-URL self-heal)");
    } else {
        // Loud: silently failing here turns every hook's self-heal off with no
        // trace, and the symptom (traffic on the retired port) looks identical
        // to nobody having fixed the hooks at all.
        tracing::warn!(?path, "could not publish endpoint.json — hooks cannot self-heal a stale AMUX_URL");
    }
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
            let mut parts = k.splitn(3, '\t');
            let ip = parts.next().unwrap_or("-");
            let ua = parts.next().unwrap_or("-");
            let session = parts.next().unwrap_or("");
            // Loudest path first, for the same reason clients are sorted that
            // way: this list is read to decide WHICH caller wearing this
            // user-agent to go and fix next.
            let mut paths: Vec<_> =
                v.paths.iter().map(|(p, c)| serde_json::json!({"path": p, "count": c})).collect();
            paths.sort_by_key(|p| std::cmp::Reverse(p["count"].as_u64().unwrap_or(0)));
            serde_json::json!({
                "ip": ip,
                "user_agent": ua,
                // "" = unattributed. That is the row that matters: an
                // attributed lane drains on its next restart, an unattributed
                // caller is code still pointing at the retired address.
                "session": session,
                "attributed": !session.is_empty(),
                "count": v.count,
                "first_seen": v.first_seen,
                "last_seen": v.last_seen,
                "last_path": v.last_path,
                "paths": paths,
                "paths_dropped": v.paths_dropped,
            })
        })
        .collect();
    // Loudest caller first: the list is read to decide who to go and fix.
    clients.sort_by_key(|c| std::cmp::Reverse(c["count"].as_u64().unwrap_or(0)));

    // THE EXIT VERDICT, computed rather than left for a reader to eyeball.
    // "Zero hits for 7 days" is unsatisfiable by waiting while pre-cutover
    // lanes are alive — their process env cannot be rotated — so the useful
    // question is not "is it zero" but "is any of this traffic a DEFECT".
    // Attributed traffic drains on its lane's next restart; unattributed
    // traffic is code that still names the retired address.
    let mut sessions: Vec<&str> = clients
        .iter()
        .filter_map(|c| c["session"].as_str())
        .filter(|s| !s.is_empty())
        .collect();
    sessions.sort_unstable();
    sessions.dedup();
    let unattributed: u64 = clients
        .iter()
        .filter(|c| !c["attributed"].as_bool().unwrap_or(false))
        .map(|c| c["count"].as_u64().unwrap_or(0))
        .sum();
    let attributed: u64 = clients
        .iter()
        .filter(|c| c["attributed"].as_bool().unwrap_or(false))
        .map(|c| c["count"].as_u64().unwrap_or(0))
        .sum();
    let elapsed = t.saturating_sub(s.started_at);
    serde_json::json!({
        "enabled": s.port != 0,
        "port": s.port,
        "canonical_port": std::env::var("AMUX_RS_PORT").unwrap_or_else(|_| crate::config::DEFAULT_PORT.to_string()),
        "attributed_hits": attributed,
        "unattributed_hits": unattributed,
        "sessions_still_on_legacy": sessions,
        // The wording is deliberately weaker than "this is a defect". Not every
        // unattributed hit is one: a request that carries no X-Amux-Session —
        // /health polls, the SPA shell, a browser — is unattributed even when
        // it comes from a pre-cutover lane that merely needs restarting.
        // Measured on the first live run of this very change: my own
        // `curl https://localhost:8822/health` landed in the unattributed
        // bucket. Claiming certainty the key cannot support is how a verdict
        // stops being read.
        "verdict": if unattributed > 0 {
            "INVESTIGATE: traffic with no X-Amux-Session. Either code still naming 8822, or header-less requests (/health, the SPA shell) from a pre-cutover lane. Read `paths` — an /api/ path with no session is the defect shape; /health is not"
        } else if !sessions.is_empty() {
            "DRAINING: every hit is an attributed pre-cutover lane whose process env cannot be rotated; it clears when those lanes restart, not by waiting"
        } else {
            "CLEAR: no traffic on the retired port this uptime"
        },
        // PERSISTED across restarts now (AMUX-2769). The old per-uptime
        // counter was chosen so a reset counter could not be misread as a
        // lifetime total — sound, but it made `ready_to_retire` unreachable:
        // it wanted 168h of window on a process the builder re-execs several
        // times an hour (316 installs, window standing at 0.71h when measured).
        // The verdict now keys on QUIET TIME instead, which a restart cannot
        // fake because a restart does not make traffic older.
        "hits_total": s.hits_total,
        "window": {
            "started_at": s.started_at,
            "hours_elapsed": elapsed as f64 / 3600.0,
            "resets_on_restart": false,
            "persisted_at": persist_path().to_string_lossy(),
        },
        "quiet_for_hours": if s.last_hit == 0 {
            serde_json::Value::Null
        } else {
            serde_json::json!(t.saturating_sub(s.last_hit) as f64 / 3600.0)
        },
        "last_hit": if s.last_hit == 0 { serde_json::Value::Null } else { s.last_hit.into() },
        "clients": clients,
        "clients_dropped": s.clients_dropped,
        "retire_when": "quiet_for_hours >= 168 (7d with no hit at all) AND clients empty this \
                        uptime -> drop AMUX_RS_LEGACY_PORT from ~/.amux/server.env and delete the \
                        bind block in lib.rs. See docs/rust-migration/server-boundary.md. \
                        Measured from a PERSISTED last_hit, so restarts no longer reset the \
                        countdown (they used to, several times an hour, which made this \
                        unreachable).",
        "ready_to_retire": s.clients.is_empty()
            && match s.last_hit {
                // Armed, never hit, and observed for a week: the genuine
                // all-clear. `started_at` is persisted too, so this is real
                // observation time rather than uptime.
                0 => elapsed >= 7 * 24 * 3600,
                last => t.saturating_sub(last) >= 7 * 24 * 3600,
            },
    })
}

/// `GET /api/debug/legacy-port`.
pub async fn debug() -> axum::Json<serde_json::Value> {
    axum::Json(snapshot())
}

/// One hour's worth of accounting, as decided rather than as logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourReport {
    pub port: u16,
    pub hits_last_hour: u64,
    pub hits_this_uptime: u64,
    /// Top callers, loudest first: `("<ip> <ua>", count)`.
    pub top: Vec<(String, u64)>,
    pub clients_dropped: u64,
    pub uptime_secs: u64,
    /// WARN vs INFO. `true` means the bind still cannot be dropped.
    pub still_in_use: bool,
}

/// Close the hour: snapshot it, reset the per-hour counter, decide the verdict.
///
/// Split out of the timer loop on purpose. A decision buried inside
/// `loop { interval.tick().await; … }` can only be tested by waiting an hour,
/// which means in practice it is not tested at all — and the thing most worth
/// pinning here is that ZERO still produces a report (see [`run_reporter`]).
///
/// `None` = the bind is not enabled, so there is nothing to say.
pub fn take_hour() -> Option<HourReport> {
    let mut s = state().lock().ok()?;
    if s.port == 0 {
        return None;
    }
    let mut top: Vec<(String, u64)> = s.clients.iter().map(|(k, v)| (k.clone(), v.count)).collect();
    top.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    top.truncate(5);
    let hits_last_hour = s.hour_hits;
    s.hour_hits = 0;
    s.hour_started = now();
    Some(HourReport {
        port: s.port,
        hits_last_hour,
        hits_this_uptime: s.hits_total,
        top,
        clients_dropped: s.clients_dropped,
        uptime_secs: now().saturating_sub(s.started_at),
        still_in_use: hits_last_hour > 0,
    })
}

/// Hourly ticker. WARN while the legacy port is still being used (naming who is
/// using it, so the line is actionable without opening the API), INFO when it is
/// not — because ZERO is the signal the retirement is waiting for, and a
/// reporter that goes silent on zero cannot be told apart from a reporter that
/// died.
pub async fn run_reporter() {
    let mut tick = tokio::time::interval(Duration::from_secs(3600));
    tick.tick().await; // fires immediately; skip the t=0 tick
    loop {
        tick.tick().await;
        let Some(r) = take_hour() else { continue };
        let uptime_hours = format!("{:.1}", r.uptime_secs as f64 / 3600.0);
        if r.still_in_use {
            let callers = r
                .top
                .iter()
                .map(|(k, c)| format!("{k} x{c}"))
                .collect::<Vec<_>>()
                .join("; ");
            tracing::warn!(
                port = r.port,
                hits_last_hour = r.hits_last_hour,
                hits_this_uptime = r.hits_this_uptime,
                uptime_hours,
                clients_dropped = r.clients_dropped,
                %callers,
                "legacy port STILL IN USE — bind cannot be dropped yet"
            );
        } else {
            tracing::info!(
                port = r.port,
                hits_last_hour = 0,
                hits_this_uptime = r.hits_this_uptime,
                uptime_hours,
                "legacy port idle — retire when this stays 0 for 7d (GET /api/debug/legacy-port)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ONE test, not several, on purpose.
    ///
    /// This module's counter is a process-global `OnceLock<Mutex<_>>` — which is
    /// correct for the thing it measures (one process, one legacy listener) and
    /// poison for tests, because cargo runs them as THREADS IN ONE PROCESS in
    /// nondeterministic order. Split across two `#[test]` fns, the "fresh state
    /// reads as disabled" assertion passed or failed depending on whether the
    /// sibling's `arm()` happened to run first: green on the first run, red on
    /// the next, with nothing changed. That exact shape is already in
    /// frustrations.md (env-mutating tests racing each other), and the fix that
    /// actually holds is not `--test-threads=1` — a flag no one passes — but
    /// keeping the sequence that shares the state inside a single test.
    ///
    /// The order below IS the assertion: never-armed, then armed-and-idle, then
    /// traffic, then the window reset.
    #[test]
    fn counter_lifecycle_disabled_then_armed_then_traffic_then_reset() {
        // (1) Never armed. The bind being enabled with zero traffic is the state
        // the retirement waits for; it must not render the same as the bind
        // being OFF, or the field the owner reads cannot distinguish "safe to
        // drop" from "already dropped, nothing was ever measured".
        let off = snapshot();
        assert_eq!(off["enabled"], false, "unarmed must report enabled:false");
        assert_eq!(off["port"], 0);

        // Point the persisted window at a throwaway file. arm() restores from
        // it (AMUX-2769), so without this the test asserts against the live
        // fleet's traffic and its verdict depends on the host.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("AMUX_LEGACY_PORT_STATE", tmp.path().join("w.json"));

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

        // (3) The hourly report must fire on ZERO, not only on traffic. This is
        // the whole design: silence is the signal the retirement waits for, so a
        // reporter that only speaks when something happened cannot be told apart
        // from one that died — and "no WARN lines lately" would read as progress
        // when it might be a dead task.
        //
        // Idle hour: reported, and flagged as NOT in use.
        let idle = take_hour().expect("armed bind must produce a report");
        assert_eq!(idle.port, 8822);
        assert_eq!(idle.hits_last_hour, 0);
        assert!(
            !idle.still_in_use,
            "a zero hour must not be flagged as in-use"
        );

        // Now drive the REAL accounting the middleware uses (`record`), not a
        // hand-written imitation of it.
        //
        // The two callers below wear the SAME user-agent and hit DIFFERENT
        // paths, because that is the case the tally could not express and the
        // drain depends on: on this machine `curl/8.7.1` is both the Claude
        // Code report hooks (`/api/sessions/*/report`) and every agent's own
        // ad-hoc `curl $AMUX_URL/...`. Fixing only the hooks must show up as
        // one path going quiet while the other does not — with a single
        // `last_path` field that is invisible, so "did my fix work?" was
        // unanswerable from what this module kept.
        assert_eq!(record("127.0.0.1", "curl/8", "", "/api/sessions/a/report"), 8822);
        record("127.0.0.1", "curl/8", "", "/api/sessions/a/report");
        record("127.0.0.1", "curl/8", "", "/api/board");
        let busy = take_hour().expect("still armed");
        assert_eq!(busy.hits_last_hour, 3);
        assert!(busy.still_in_use, "a non-zero hour must be flagged in-use");
        assert_eq!(
            busy.top.first().map(|(k, c)| (k.as_str(), *c)),
            Some(("127.0.0.1\tcurl/8\t", 3)),
            "the WARN line must be able to name the caller"
        );

        // THE DISCRIMINATOR. One client row, two callers, and the snapshot has
        // to separate them by path. This assertion fails on the pre-change
        // module (there was no `paths` key at all), which is what makes it a
        // check rather than decoration.
        let snap = snapshot();
        let row = snap["clients"]
            .as_array()
            .and_then(|c| c.iter().find(|c| c["user_agent"] == "curl/8"))
            .expect("the curl caller must appear in the snapshot")
            .clone();
        let paths: Vec<(String, u64)> = row["paths"]
            .as_array()
            .expect("per-client path histogram")
            .iter()
            .map(|p| (p["path"].as_str().unwrap_or("").to_string(), p["count"].as_u64().unwrap_or(0)))
            .collect();
        assert_eq!(
            paths,
            vec![("/api/sessions/a/report".to_string(), 2), ("/api/board".to_string(), 1)],
            "paths must be split per caller and sorted loudest-first — one row \
             collapsing both callers is the state that made the drain unmeasurable"
        );
        assert_eq!(row["paths_dropped"], 0);

        // The window RESET is what makes "last hour" mean last hour; without it
        // the count would only ever climb and every hour after the first would
        // read as in-use forever.
        let after = take_hour().expect("still armed");
        assert_eq!(
            after.hits_last_hour, 0,
            "take_hour must reset the per-hour counter"
        );
        assert_eq!(
            after.hits_this_uptime, 3,
            "the uptime total must NOT reset — only the hour window does"
        );
        assert!(!after.still_in_use);

        // THE EXIT DISCRIMINATOR (AMUX-2769). Everything above is unattributed
        // traffic, so the verdict must say BLOCKED — that is code naming the
        // retired port and it is a defect someone has to go and fix.
        let snap = snapshot();
        assert_eq!(snap["unattributed_hits"], 3);
        assert_eq!(snap["attributed_hits"], 0);
        assert!(
            snap["verdict"].as_str().unwrap_or("").starts_with("INVESTIGATE"),
            "unattributed traffic must not read as merely draining"
        );

        // Now an ATTRIBUTED lane — a pre-cutover session whose process env
        // still says 8822. Same ip, same user-agent: on the OLD key these were
        // byte-identical to the rows above and the whole fleet collapsed into
        // one number, which is exactly why "zero hits for 7 days" could never
        // be reasoned about.
        record("127.0.0.1", "curl/8", "mixpeek-studio", "/api/board");
        record("127.0.0.1", "curl/8", "amux", "/api/board");
        let snap = snapshot();
        assert_eq!(snap["attributed_hits"], 2);
        assert_eq!(
            snap["sessions_still_on_legacy"],
            serde_json::json!(["amux", "mixpeek-studio"]),
            "the lanes to restart must be NAMED — an ip+ua tally cannot name them"
        );
        // Unattributed traffic still outranks: a defect is not cleared by a
        // lane that merely needs restarting.
        assert!(snap["verdict"].as_str().unwrap_or("").starts_with("INVESTIGATE"));

        // (5) THE COUNTDOWN MUST SURVIVE A RESTART (AMUX-2769). `ready_to_retire`
        // wanted 168h of observation on a process the builder re-execs several
        // times an hour — measured 2026-08-11: 316 installs, window standing at
        // 0.71h. The retirement this module exists to decide could never be
        // reached, and nothing said so: the field just read `false` forever,
        // which is indistinguishable from "still busy".
        //
        // Folded into THIS test rather than written as its own, for the exact
        // reason the doc above gives: arm() mutates shared state, so a second
        // arming test is order-dependent across threads. It failed that way on
        // the first run.
        let long_ago = now().saturating_sub(8 * 24 * 3600);
        save_persisted(long_ago, 42, long_ago);
        arm(8822); // "restart"
        let back = snapshot();
        assert_eq!(back["hits_total"], 42, "a restart must not forget prior traffic");
        assert_eq!(
            back["window"]["resets_on_restart"], false,
            "the field must stop claiming a reset it no longer does"
        );
        let quiet = back["quiet_for_hours"].as_f64().expect("quiet_for_hours");
        assert!(quiet >= 168.0, "8 days of silence must read as >=168h, got {quiet}");
        assert_eq!(
            back["ready_to_retire"], true,
            "no hit for 8 days and no clients this uptime IS the all-clear"
        );

        // A fresh hit must revoke it — a one-way latch would be worse than a
        // verdict that never fires.
        record("127.0.0.1", "curl/8", "", "/api/board");
        let after = snapshot();
        assert_eq!(after["ready_to_retire"], false, "a hit must revoke the all-clear");
        assert!(after["quiet_for_hours"].as_f64().unwrap_or(999.0) < 1.0);
    }
}
