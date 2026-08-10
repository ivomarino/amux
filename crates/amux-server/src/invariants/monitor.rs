//! The periodic driver (AMUX-2622).
//!
//! Binds the pure checks in [`super::checks`] to real system state and records
//! the results. The checks stay pure and table-driven precisely so their
//! negative controls can inject failures without a live fleet; this module is
//! the only place that touches the world.

use super::{checks, store, Confidence, InvariantResult};
use crate::api::AppState;
use serde_json::json;

/// Tick interval. 30s is chosen against the spec's SLOs (§31: backend/DB worker
/// drift < 10s, stuck command < 30s) for the checks that are cheap; anything
/// needing faster detection belongs on a post-mutation hook, not here. A poll
/// interval is a ceiling on detection latency, never a substitute for a
/// postcondition at the mutation site.
const TICK_SECS: u64 = 30;

/// Run every registered invariant once against live state.
///
/// Returns the results rather than only persisting them so the HTTP handler can
/// serve a FRESH evaluation on demand — a health endpoint that can only replay
/// the last poll cannot answer "is it broken right now".
pub async fn evaluate_all(state: &AppState) -> Vec<InvariantResult> {
    let mut out = Vec::new();

    // -- 1. route contract: do shipped clients call routes that exist?
    let mounted: Vec<(&str, &[&str])> = crate::api::request_log::ROUTE_TABLE
        .iter()
        .map(|e| (e.path, e.methods))
        .collect();
    let callers = extract_caller_paths();
    out.extend(checks::route_callers_have_routes(&mounted, &callers));

    // -- 2. config provenance: did server.env reach the process?
    //
    // Reads the FILE and compares against the live process env. Deliberately
    // not against `ServerConfig.env`, which is the merged in-memory view and
    // would agree with itself by construction: the incident was that
    // `std::env::var()` call sites saw nothing while the config struct looked
    // correct, so the config struct is exactly the wrong oracle here.
    let env_path = crate::config::ServerConfig::from_process_env()
        .amux_home
        .join("server.env");
    match std::fs::read_to_string(&env_path) {
        Ok(text) => out.extend(checks::config_env_reaches_process(&text, &|k| {
            std::env::var(k).ok()
        })),
        Err(e) => out.push(InvariantResult::unknown(
            "config.env_reaches_process",
            format!("server.env unreadable: {e}"),
        )),
    }

    // -- 3. queue liveness: is anything queued in front of an IDLE target?
    out.extend(steering_queue_check(state).await);

    // -- 4. status truth: does the card agree with the pane?
    out.extend(status_pane_check(state));

    out
}

/// The derived card status against the physical pane, per lane (AMUX-2646).
///
/// Reads BOTH sides through `FleetSignals` — the same struct, the same
/// capture, the same detectors the derivation itself uses. Re-deriving either
/// side here would produce a check that can disagree with the mechanism it
/// audits, which is the failure this whole module exists to catch.
///
/// Cost is bounded by `capture_panes`, which probes only lanes that painted
/// inside the contradiction window: 4 of 63 on the fleet this was measured on.
fn status_pane_check(state: &AppState) -> Vec<InvariantResult> {
    const ID: &str = "status.agrees_with_pane";
    let Ok(conn) = state.store.read() else {
        return vec![InvariantResult::unknown(ID, "store unreadable")];
    };
    let mut signals = crate::api::sessions_legacy::FleetSignals::load(&conn);
    if signals.running.is_empty() {
        // No tmux fleet is a real state (a fresh box), but it is also exactly
        // what a failed `tmux list-sessions` looks like — and that has shipped
        // here before, serving running=0 for 116 live cards. Do not call it a
        // pass.
        return vec![InvariantResult::unknown(ID, "no running tmux sessions visible")];
    }
    signals.capture_panes();
    let lanes: Vec<checks::LaneTruth> = signals
        .probed_lanes()
        .into_iter()
        .map(|(name, pane_says_working)| {
            let rep = signals.reports.get(&name).cloned().unwrap_or(json!({}));
            checks::LaneTruth {
                status: signals.derive_status(&name, true),
                pane_says_working,
                report_state: rep["state"].as_str().unwrap_or("").into(),
                report_age_s: signals.now - rep["ts"].as_f64().unwrap_or(signals.now),
                report_source: rep["source"].as_str().unwrap_or("").into(),
                report_origin: rep["origin"].as_str().unwrap_or("").into(),
                name,
            }
        })
        .collect();
    checks::status_agrees_with_pane(&lanes)
}

/// The steering queue, joined against each target's reported state.
///
/// Reads the same `session_reports` blob the delivery gate reads, so the check
/// and the mechanism it audits cannot disagree about what "idle" means — the
/// ethos rule about a view sharing the predicate of the mechanism it describes.
async fn steering_queue_check(state: &AppState) -> Vec<InvariantResult> {
    const ID: &str = "queue.has_live_consumer";
    let Ok(conn) = state.store.read() else {
        return vec![InvariantResult::unknown(ID, "store unreadable")];
    };
    let reports: serde_json::Value = conn
        .query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));

    let Ok(mut stmt) = conn.prepare(
        "SELECT session, MIN(queued_at) FROM steering_queue GROUP BY session",
    ) else {
        // The table not existing is a real answer (nothing queued), but an
        // unreadable one is not — do not turn a failed read into a clean pass.
        return vec![InvariantResult::unknown(ID, "steering_queue unreadable")];
    };
    let items: Vec<checks::QueuedItem> = stmt
        .query_map([], |r| {
            let session: String = r.get(0)?;
            let queued_at: f64 = r.get(1)?;
            Ok((session, queued_at))
        })
        .map(|it| {
            it.flatten()
                .map(|(session, queued_at)| {
                    let idle = reports[&session]["state"].as_str() == Some("idle");
                    checks::QueuedItem {
                        queue: "steering".into(),
                        target: session,
                        queued_at,
                        target_idle: idle,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    // 300s: comfortably more than several delivery ticks, so a normal
    // busy->idle transition never trips it, but far below the 2h6m the real
    // incident reached.
    checks::queue_has_live_consumer(&items, now, 300.0)
}

/// Client call sites, extracted from the shipped artifacts.
///
/// Sourced from the EMBEDDED dashboard bytes rather than from disk: the binary
/// serves what it embedded, so checking a file on disk would audit a different
/// artifact than the one users load — the same class of mistake as verifying a
/// fix against a file the server is not running.
fn extract_caller_paths() -> Vec<checks::CallerPath> {
    let mut out = Vec::new();
    if let Some(js) = amux_dashboard::DashboardAssets::get("app.js") {
        let text = String::from_utf8_lossy(&js.data);
        out.extend(scan_js_calls(&text, "spa:app.js"));
    }
    out
}

/// Pull `API + '/api/...'` call sites out of the SPA, with their method.
///
/// Conservative by construction: only literal paths are extracted, and a path
/// built by interpolation is skipped rather than guessed at. A guessed path
/// would produce a phantom failure, and a check that cries wolf is one people
/// turn off — worse than the gap it was covering.
fn scan_js_calls(js: &str, source: &str) -> Vec<checks::CallerPath> {
    // Byte offsets from `find` are not necessarily char boundaries, and app.js
    // is full of box-drawing characters in comments. Slicing blind panicked on
    // the real bundle — caught by this module's own extractor test, which is
    // the argument for having one.
    let clamp = |i: usize| -> usize {
        let mut i = i.min(js.len());
        while i > 0 && !js.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = js[i..].find("'/api/") {
        let start = i + p + 1;
        let Some(endrel) = js[start..].find('\'') else { break };
        let end = start + endrel;
        let raw = &js[start..end];
        i = end;
        // Interpolated or query-bearing paths: keep the literal prefix only,
        // and skip if that leaves nothing addressable. Guessing at an
        // interpolated path produces phantom failures, and a check that cries
        // wolf gets turned off.
        let path = raw.split(['?', '$', '`']).next().unwrap_or("").trim_end_matches('/');
        if path.len() < 5 || !path.starts_with("/api/") {
            continue;
        }
        // Is the literal followed by concatenation/interpolation? Then `path`
        // is a PREFIX (`'/api/board/' + id`) and must be matched leniently.
        // Also true when the literal itself ended in '/' or carried a template
        // marker. Getting this wrong is not a stricter check but a wrong one —
        // it produced 86 false failures on the first live run.
        let tail = &js[clamp(end)..clamp(end + 12)];
        let interpolated = raw.ends_with('/')
            || raw.contains('$')
            || raw.contains('`')
            || tail.trim_start().starts_with('+');
        // Method literal, looking FORWARD first: `fetch(API + '/x', {method:
        // 'POST'})` puts it after the URL, which is the overwhelmingly common
        // shape. A backward-only window read every POST as a GET and would
        // have made the whole 405 class invisible — the exact bug this census
        // exists to catch. Backward is still consulted for the
        // `const opts = {method:'PATCH'}; fetch(url, opts)` shape.
        let fwd = &js[clamp(end)..clamp(end + 200)];
        let back = &js[clamp(start.saturating_sub(200))..clamp(start)];
        let find_m = |hay: &str| {
            ["POST", "PATCH", "DELETE", "PUT"]
                .iter()
                .find(|m| {
                    hay.contains(&format!("'{m}'")) || hay.contains(&format!("\"{m}\""))
                })
                .map(|m| m.to_string())
        };
        let method = find_m(fwd).or_else(|| find_m(back)).unwrap_or_else(|| "GET".into());
        out.push(checks::CallerPath {
            method,
            path: path.to_string(),
            source: source.to_string(),
            interpolated,
        });
    }
    out.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
    out.dedup_by(|a, b| a.path == b.path && a.method == b.method && a.interpolated == b.interpolated);
    out
}

/// One evaluation pass: check, persist, reconcile incidents.
pub async fn tick(state: &AppState) -> (Confidence, usize) {
    let t0 = std::time::Instant::now();
    let results = evaluate_all(state).await;
    let conf = super::rollup(&results);
    let opened = store::record(&state.store, results, t0.elapsed().as_millis() as i64).await;
    if opened > 0 {
        tracing::warn!(opened, confidence = conf.as_str(), "invariant incidents opened");
    }
    (conf, opened)
}

/// Background driver.
pub async fn run(state: AppState) {
    // Stagger past boot so the first pass sees a settled process rather than
    // half-initialised state, which would open incidents that immediately heal
    // and teach everyone the monitor is noisy.
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    loop {
        let st = state.clone();
        // A panic in one pass must not kill the monitor for the process
        // lifetime — a dead monitor is the failure this whole module exists to
        // make visible, so it must not be able to die quietly itself.
        if let Err(e) = tokio::spawn(async move { tick(&st).await }).await {
            tracing::error!(error = %e, "invariant tick panicked");
        }
        tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The BINDING, not the check. `status_agrees_with_pane` has negative
    /// controls of its own, but a pure check that `evaluate_all` never calls is
    /// a check nobody runs — the "capability that exists but reaches nobody"
    /// failure, one layer down. This asserts the wiring produces a verdict.
    ///
    /// Machine-independent by construction: on a box with a live tmux fleet it
    /// returns one result per probed lane, on one without it returns a single
    /// `Unknown`. What it may never do is return NOTHING, which is what a
    /// silently-dropped binding looks like.
    #[test]
    fn the_status_pane_check_is_actually_wired_into_the_monitor() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::db::Store::open(&dir.path().join("t.db")).unwrap();
        let state = AppState {
            store: std::sync::Arc::new(store),
            started: std::time::Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        let rs = status_pane_check(&state);
        assert!(!rs.is_empty(), "the binding must always reach a verdict");
        assert!(
            rs.iter().all(|r| r.invariant_id == "status.agrees_with_pane"),
            "wrong invariant id — the sweep contract greps for this exact string"
        );
    }

    /// The SPA extractor must find real call sites in the shipped bundle. If it
    /// returns nothing the census silently covers nothing — the empty-probe
    /// trap this repo has hit repeatedly — so this is a control on the
    /// EXTRACTOR, not on the fleet.
    #[test]
    fn the_spa_extractor_finds_real_call_sites() {
        let calls = extract_caller_paths();
        assert!(
            calls.len() > 20,
            "expected many /api/ call sites in app.js, got {} — extractor is broken",
            calls.len()
        );
        assert!(
            calls.iter().any(|c| c.path.starts_with("/api/board")),
            "the SPA certainly calls /api/board; not finding it means the scan is wrong"
        );
    }

    /// Interpolated paths must be skipped, not guessed — a phantom failure
    /// trains people to ignore the check.
    #[test]
    fn interpolated_paths_are_not_guessed() {
        let js = r#"fetch(API + '/api/workers/' + name + '/send', {method:'POST'})"#;
        let calls = scan_js_calls(js, "t");
        assert!(
            calls.iter().all(|c| !c.path.contains("${") && !c.path.contains('`')),
            "must not emit interpolation fragments as paths"
        );
    }

    /// Method detection must see an explicit POST rather than defaulting to GET,
    /// or every verb-missing bug (the 405 class) is invisible to the census.
    #[test]
    fn explicit_method_is_detected() {
        let js = r#"await fetch(API + '/api/board', {method:'POST', body:x})"#;
        let calls = scan_js_calls(js, "t");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST", "a POST read as GET hides 405s");
    }
}
