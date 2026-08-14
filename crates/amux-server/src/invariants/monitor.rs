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
/// pub so lib.rs registers the cadence this loop actually sleeps with
/// `runtime_jobs::registry`, instead of a second copy of the number.
pub const TICK_SECS: u64 = 30;

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

    // -- 1b. no two lanes share a Claude conversation (AMUX-1730 / AMUX-2819).
    //
    // Reads the session meta files, which are the same store the resume path
    // reads `cc_conversation_id` from — so this cannot disagree with the thing
    // it describes.
    {
        let sessions_dir = crate::config::ServerConfig::from_process_env()
            .amux_home
            .join("sessions");
        let mut pairs: Vec<(String, String)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
            for e in rd.flatten() {
                let p = e.path();
                let Some(fname) = p.file_name().and_then(|f| f.to_str()) else { continue };
                let Some(name) = fname.strip_suffix(".meta.json") else { continue };
                let Ok(text) = std::fs::read_to_string(&p) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                if let Some(c) = v.get("cc_conversation_id").and_then(|c| c.as_str()) {
                    if !c.trim().is_empty() {
                        pairs.push((name.to_string(), c.trim().to_string()));
                    }
                }
            }
        }
        out.extend(checks::conversations_are_not_shared(&pairs));
    }

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

    // -- 5. is the report control plane up? (2026-08-13 fleet-wide outage)
    out.extend(self_reports_check(state));

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
    // Both checks read the SAME `lanes` (one capture, one derivation) so they
    // cannot disagree with each other or with the mechanism they audit. The
    // first flags a working pane read idle; the second (AMUX-3047) flags the
    // inverse sharp case — `active` derived over a FRESH idle self-report and a
    // quiet pane, i.e. the harness's own report being overridden.
    let mut results = checks::status_agrees_with_pane(&lanes);
    results.extend(checks::status_contradicts_fresh_idle_report(&lanes));
    results
}

/// Is the report control plane UP — are self-reports landing at all?
///
/// Reads the SAME `FleetSignals.reports` blob the status derivation reads, so
/// this cannot disagree with the mechanism it audits about what "reported"
/// means. The discriminator is the FLEET MINIMUM report age, not any per-lane
/// age: one idle lane going quiet for hours is normal, the youngest report
/// across the whole fleet being hours old is the control plane down (the
/// 2026-08-13 outage, where baked-in report hooks POSTed to the dead 8822).
fn self_reports_check(state: &AppState) -> Vec<InvariantResult> {
    const ID: &str = "session.self_reports_landing";
    let Ok(conn) = state.store.read() else {
        return vec![InvariantResult::unknown(ID, "store unreadable")];
    };
    let signals = crate::api::sessions_legacy::FleetSignals::load(&conn);
    if signals.running.is_empty() {
        // Same reasoning as status_pane_check: no fleet is a real state but also
        // what a failed `tmux list-sessions` looks like. Not a pass.
        return vec![InvariantResult::unknown(ID, "no running tmux sessions visible")];
    }
    // NAMESPACE: `signals.running` holds the tmux session names, which are the
    // `amux-<n>` form; `signals.reports` is keyed by the BARE `AMUX_SESSION`
    // name (`<n>`) the report POST carries. `probed_lanes()` bridges the two
    // with `format!("amux-{n}")` — do the same here, or every lookup misses and
    // the check reports "0 of N reporting" against a blob that is actually full
    // (caught immediately on first deploy, 2026-08-13). `agent_running` also
    // drops shell-only panes, which are not lanes and never report.
    let lanes: Vec<checks::LaneReport> = signals
        .running
        .iter()
        .filter_map(|t| t.strip_prefix("amux-"))
        .filter(|n| signals.agent_running(&format!("amux-{n}")))
        .map(|n| {
            let age = signals
                .reports
                .get(n)
                .and_then(|r| r["ts"].as_f64())
                .map(|ts| signals.now - ts);
            checks::LaneReport { name: n.to_string(), report_age_s: age }
        })
        .collect();
    // Policy in config, not baked in (ethos D4). Defaults: a fleet of >=10 lanes
    // with NObody reporting in an hour is unambiguously broken — a healthy fleet
    // transitions within minutes, and the incident's freshest was 2h.
    let min_lanes = std::env::var("AMUX_REPORT_MIN_LANES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let max_freshest_s = std::env::var("AMUX_REPORT_FRESHEST_MAX_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600.0);
    checks::self_reports_landing(&lanes, min_lanes, max_freshest_s)
}

/// The steering queue, joined against each target's reported state.
///
/// Reads the same `session_reports` blob the delivery gate reads, so the check
/// and the mechanism it audits cannot disagree about what "idle" means — the
/// ethos rule about a view sharing the predicate of the mechanism it describes.
async fn steering_queue_check(state: &AppState) -> Vec<InvariantResult> {
    const ID: &str = "queue.has_live_consumer";
    // Read everything from the store in a scope that ENDS before any await: the
    // rusqlite Connection guard and Statement are !Send, so they must be fully
    // out of scope (not merely dropped) before lane_block_reason's tmux await,
    // or the whole invariant future stops being Send. This also releases the
    // read lock before that terminal I/O rather than holding it across the await.
    let (reports, rows): (serde_json::Value, Vec<(String, f64)>) = {
        let Ok(conn) = state.store.read() else {
            return vec![InvariantResult::unknown(ID, "store unreadable")];
        };
        let reports = conn
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
            // unreadable one is not; do not turn a failed read into a clean pass.
            return vec![InvariantResult::unknown(ID, "steering_queue unreadable")];
        };
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        (reports, rows)
    };

    let mut items: Vec<checks::QueuedItem> = Vec::with_capacity(rows.len());
    for (session, queued_at) in rows {
        let idle = reports[&session]["state"].as_str() == Some("idle");
        // block_reason is the SAME predicate the delivery loop gates on
        // (session_verbs::lane_block_reason), so the check cannot disagree with
        // the mechanism about ROUTABILITY either, which is the missing half that
        // made a renamed-away ghost target read as a dead consumer (AMUX-3084).
        let block_reason = crate::api::session_verbs::lane_block_reason(&session)
            .await
            .map(str::to_string);
        items.push(checks::QueuedItem {
            queue: "steering".into(),
            target: session,
            queued_at,
            target_idle: idle,
            block_reason,
        });
    }

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
    // THE CLI HALF (AMUX-2917). CallerPath::source has documented
    // `"spa:app.js" / "cli:amux"` since this check was written, and CLAUDE.md's
    // observability table describes the invariant as enumerating "SPA/CLI call
    // sites" — but only the SPA was ever scanned. The CLI is the fleet's other
    // real client (every `amux board`, `amux send`, `amux crm` is a curl), so
    // half the callers were outside the only check that can name an unrouted
    // one.
    //
    // Embedded at BUILD time, like app.js, deliberately: reading it off disk
    // would check whatever `amux` happens to be sitting in the checkout —
    // possibly a peer's mid-edit — instead of the source this binary was built
    // from. Same reason the e2e harness builds HEAD (AMUX-2924).
    out.extend(scan_shell_calls(include_str!("../../../../amux"), "cli:amux"));
    out
}

/// Pull `"$AMUX_URL/api/..."` call sites out of the bash CLI, with their method.
///
/// METHOD IS KNOWABLE HERE, unlike in the SPA, because curl's own rules decide
/// it: an explicit `-X VERB` wins, otherwise `-d`/`--data`/`--data-binary`
/// means POST, otherwise GET. That is not a heuristic, it is curl's documented
/// behaviour — so these call sites carry `method_known: true` and a genuine
/// 405 (route exists, wrong verb) is detectable in the CLI as well.
///
/// Anchored on the `curl` token rather than on the path, and that is the whole
/// accuracy story: this file is 2400 lines of shell in which `/api/...` also
/// appears inside help text, echoed examples and comments. Requiring a `curl`
/// within the preceding window is what keeps a printed example (`echo "  curl
/// -sk \"$AMUX_URL/api/workers/...\""`) from being reported as a live caller —
/// though an echoed example that really is malformed will still be caught,
/// which is a feature.
fn scan_shell_calls(sh: &str, source: &str) -> Vec<checks::CallerPath> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(p) = sh[i..].find("/api/") {
        let start = i + p;
        // The literal runs until anything that ends a shell word or starts an
        // expansion. `$` stops it because `/api/board/$id` is a PREFIX, not a
        // path — same rule the JS scanner uses, for the same reason.
        let rest = &sh[start..];
        let endrel = rest
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '"' | '\'' | '`' | '$' | '?' | '\\' | ')' | ';' | '|' | '>')
            })
            .unwrap_or(rest.len());
        let raw = &rest[..endrel];
        i = start + endrel.max(1);

        let path = raw.trim_end_matches('/');
        if path.len() < 5 || !path.starts_with("/api/") {
            continue;
        }
        // A GLOB IS DOCUMENTATION, NOT A CALL SITE. `amux crm help` prints
        // "HTTP: /api/crm/*", and the curl anchor does NOT save it — the help
        // heredoc sits within 600 chars of the branch's real curls. So the
        // anchor narrows the phantom class, it does not eliminate it, and the
        // remaining shapes have to be named. `*` cannot appear in a request
        // path this server routes, so a literal containing one is prose.
        //
        // Caught by dumping what the scanner actually reported instead of
        // trusting the failure count: the census was clean, because the SPA
        // catch-all happens to match `/api/crm/*` today. A phantom that is
        // currently harmless is still a phantom, and it would have surfaced as
        // a false failure the moment that matching changed.
        if path.contains('*') || path.contains('{') || path.contains('%') {
            continue;
        }
        // Interpolated if the literal was cut short by an expansion or ended in
        // a slash — then it is a prefix and must match leniently.
        let cut_char = rest[endrel..].chars().next();
        let interpolated = raw.ends_with('/') || matches!(cut_char, Some('$') | Some('`'));

        // Backward to the anchoring `curl`. Bash puts flags BEFORE the URL, so
        // backward is correct here — the opposite of the SPA, where the method
        // literal follows the URL.
        let win_start = start.saturating_sub(600);
        let mut win_start = win_start;
        while win_start > 0 && !sh.is_char_boundary(win_start) {
            win_start -= 1;
        }
        let window = &sh[win_start..start];
        let Some(curl_at) = window.rfind("curl") else { continue };
        let cmd = &window[curl_at..];

        let method = if let Some(x) = cmd.find("-X ") {
            cmd[x + 3..]
                .split_whitespace()
                .next()
                .unwrap_or("GET")
                .trim_matches(|c: char| !c.is_ascii_alphabetic())
                .to_uppercase()
        } else if cmd.contains(" -d ") || cmd.contains("--data") {
            // curl: a body without an explicit verb is a POST.
            "POST".to_string()
        } else {
            "GET".to_string()
        };
        if method.is_empty() {
            continue;
        }
        out.push(checks::CallerPath {
            method,
            path: path.to_string(),
            source: source.to_string(),
            interpolated,
            method_known: true,
        });
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
        // BOUNDED TO THE SAME STATEMENT. A flat 200-char window bleeds past the
        // end of this call and attaches a neighbour's verb: `fetch('/api/
        // layout-presets')` (a GET) sat within 200 chars of a later
        // `{method:'DELETE'}` and was reported as `DELETE /api/layout-presets`
        // — a route that does not exist, while the DELETE the client really
        // makes (`/api/layout-presets/{name}`) is mounted and fine. One
        // confirmed false positive in the 2026-08-11 census, on a check whose
        // entire output is a work list. The options object lives inside the
        // same statement, so stopping at the first `;` keeps every real shape
        // and drops the bleed.
        let fwd_raw = &js[clamp(end)..clamp(end + 200)];
        let fwd = fwd_raw.split(';').next().unwrap_or(fwd_raw);
        // BOUNDED THE SAME WAY, and for the same reason. Bounding only forward
        // left the bug alive by the other door: app.js:3826's plain GET picked
        // up the `{method:'DELETE'}` from the DIFFERENT call six lines earlier
        // and was still reported as `DELETE /api/layout-presets`. Take only the
        // text after the previous `;` — the current statement.
        //
        // The shape this fallback was kept for (`const opts = {method:'PATCH'};
        // fetch(url, opts)`) does not occur in app.js: the one indirect call
        // (apiCall, :1842) passes a VARIABLE url, so this extractor — which
        // scans for literal '/api/...' strings — never reaches it. The fallback
        // was buying nothing and costing a phantom row.
        let back_raw = &js[clamp(start.saturating_sub(200))..clamp(start)];
        let back = back_raw.rsplit(';').next().unwrap_or(back_raw);
        let find_m = |hay: &str| {
            ["POST", "PATCH", "DELETE", "PUT"]
                .iter()
                .find(|m| {
                    hay.contains(&format!("'{m}'")) || hay.contains(&format!("\"{m}\""))
                })
                .map(|m| m.to_string())
        };
        let observed = find_m(fwd).or_else(|| find_m(back));
        let method_known = observed.is_some();
        let method = observed.unwrap_or_else(|| "GET".into());
        out.push(checks::CallerPath {
            method,
            path: path.to_string(),
            source: source.to_string(),
            interpolated,
            method_known,
        });
    }
    out.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
    out.dedup_by(|a, b| {
        a.path == b.path && a.method == b.method && a.interpolated == b.interpolated
    });
    out
}

/// One evaluation pass: check, persist, reconcile incidents.
pub async fn tick(state: &AppState) -> (Confidence, usize) {
    let t0 = std::time::Instant::now();
    let results = evaluate_all(state).await;
    let conf = super::rollup(&results);
    // Publish for /health (AMUX-2625): the endpoint everyone polls reads this
    // cached verdict rather than re-running the suite per request.
    super::record_confidence(conf, chrono::Utc::now().timestamp() as f64);
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
        crate::runtime_jobs::registry::tick(crate::runtime_jobs::registry::ids::INVARIANTS);
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
        // status_pane_check now wires TWO checks over the SAME lanes: the
        // pane-agreement check and its sharper inverse (AMUX-3047,
        // status.contradicts_fresh_idle_report). Which ids appear is
        // machine-dependent — a box with no tmux fleet early-returns a single
        // `status.agrees_with_pane` Unknown before the lanes (and so the second
        // check) are built, while a box with a live fleet emits both — so this
        // asserts the binding reaches a verdict with NO id outside the expected
        // pair (a third id leaking in is the failure). The second check's own
        // discrimination is proven machine-independently in
        // `checks::tests::fresh_idle_report_contradiction_*`.
        assert!(
            rs.iter().all(|r| {
                r.invariant_id == "status.agrees_with_pane"
                    || r.invariant_id == "status.contradicts_fresh_idle_report"
            }),
            "unexpected invariant id — the sweep contract greps for these exact strings"
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
    /// A verb belonging to a LATER call must not be attached to this one.
    /// The flat 200-char forward window did exactly that and produced a
    /// confirmed false positive in the live census (2026-08-11): a plain GET
    /// reported as `DELETE /api/layout-presets`, a path that is not mounted,
    /// while the real DELETE goes to /api/layout-presets/{name} and works.
    /// A URL built into a variable puts the verb outside the URL's own
    /// statement, so no method is observable. The extractor must SAY so rather
    /// than default to GET and let the census file a phantom 405 — which is
    /// what `GET /api/dictate` was, while the real call five lines down is a
    /// POST.
    #[test]
    fn a_defaulted_verb_is_marked_as_not_observed() {
        let js = "const url = API + '/api/dictate?session=' + s;\n\
                  const r = await fetch(url, { method: 'POST' });";
        let got = scan_js_calls(js, "t");
        let d: Vec<_> = got.iter().filter(|c| c.path == "/api/dictate").collect();
        assert!(!d.is_empty(), "the path must still be extracted");
        for c in &d {
            assert!(!c.method_known, "no verb is in this statement — it must not be claimed as observed");
        }
        // A verb in the SAME statement is still observed.
        let js2 = "await fetch(API + '/api/dictate', {method:'POST'});";
        let got2 = scan_js_calls(js2, "t");
        let d2: Vec<_> = got2.iter().filter(|c| c.path == "/api/dictate").collect();
        assert!(d2.iter().all(|c| c.method_known && c.method == "POST"), "{got2:?}");
    }

    #[test]
    fn a_neighbours_method_is_not_attached_to_this_call() {
        let js = "const r = await fetch('/api/layout-presets');\n\
                  async function del(name) {\n\
                    await fetch('/api/layout-presets/' + name, {method:'DELETE'});\n\
                  }";
        let got = scan_js_calls(js, "t");
        let base: Vec<_> = got.iter().filter(|c| c.path == "/api/layout-presets" && !c.interpolated).collect();
        assert!(!base.is_empty(), "the plain GET must still be extracted");
        for c in &base {
            assert_eq!(c.method, "GET", "a later {{method:'DELETE'}} must not become this call's verb");
        }
        // ...and the real DELETE is still found, as an interpolated prefix.
        assert!(
            got.iter().any(|c| c.path == "/api/layout-presets" && c.interpolated && c.method == "DELETE"),
            "the parameterised DELETE must still be extracted: {got:?}"
        );
    }

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


#[cfg(test)]
mod shell_scanner_tests {
    use super::*;

    /// curl's own rules, which is why these carry method_known=true: an
    /// explicit -X wins; a body without one is a POST; otherwise GET.
    #[test]
    fn the_method_comes_from_curls_rules_not_a_guess() {
        let sh = r#"
          curl -sk "$AMUX_URL/api/board"
          curl -sk -X PATCH -H 'x: y' -d "$body" "$AMUX_URL/api/prefs"
          curl -sk -d "$json" "$AMUX_URL/api/alert/owner"
          curl -sk -X DELETE "$AMUX_URL/api/schedules/SCHED-1"
        "#;
        let got: Vec<(String, String)> =
            scan_shell_calls(sh, "t").into_iter().map(|c| (c.method, c.path)).collect();
        assert_eq!(
            got,
            vec![
                ("GET".into(), "/api/board".into()),
                ("PATCH".into(), "/api/prefs".into()),
                ("POST".into(), "/api/alert/owner".into()),
                ("DELETE".into(), "/api/schedules/SCHED-1".into()),
            ]
        );
    }

    /// The anchor is what stops help text and comments being reported as live
    /// callers. Without it this file's 2400 lines of shell would produce
    /// phantom failures, and a check that cries wolf gets turned off.
    #[test]
    fn a_path_with_no_curl_in_front_of_it_is_not_a_caller() {
        let sh = r#"
          # see also /api/does-not-exist for the old contract
          echo "  try: $AMUX_URL/api/also-not-real"
        "#;
        assert!(scan_shell_calls(sh, "t").is_empty(), "comments and echoes are not call sites");
    }

    /// `$` cuts the literal, so `/api/board/$id` is a PREFIX. Treating it as an
    /// exact path is what produced 86 false failures on the SPA scanner's first
    /// live run.
    #[test]
    fn an_expansion_makes_the_path_a_prefix() {
        let got = scan_shell_calls(r#"curl -sk "$AMUX_URL/api/board/$id""#, "t");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/api/board");
        assert!(got[0].interpolated, "an expansion means match leniently");
    }

    /// Documentation shapes are not call sites, even with a curl nearby — the
    /// anchor narrows the phantom class but does not eliminate it.
    #[test]
    fn a_glob_in_the_path_is_prose_not_a_caller() {
        let sh = r#"
          curl -sk "$AMUX_URL/api/crm/contacts"
          cat <<'EOH'
          amux crm — contacts (HTTP: /api/crm/*)
EOH
        "#;
        let got = scan_shell_calls(sh, "t");
        assert!(
            got.iter().all(|c| !c.path.contains('*')),
            "a glob is documentation: {:?}",
            got.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
        assert!(got.iter().any(|c| c.path == "/api/crm/contacts"), "the real call still counts");
    }

    /// The real CLI must yield real call sites — an extractor that finds
    /// nothing is broken, not vindicated (the empty-grep trap).
    #[test]
    fn the_real_cli_yields_call_sites() {
        let found = scan_shell_calls(include_str!("../../../../amux"), "cli:amux");
        assert!(found.len() > 20, "only {} call sites scraped from the CLI", found.len());
        assert!(
            found.iter().any(|c| c.path.starts_with("/api/board")),
            "the CLI certainly calls /api/board"
        );
    }
}

#[cfg(test)]
mod extractor_wiring_tests {
    /// The CLI scan must actually be WIRED into the census, not merely exist.
    /// A scanner nobody calls and a codebase with no CLI defects produce the
    /// identical result — zero new failures — so the count alone cannot tell
    /// them apart (ethos rule 4).
    #[test]
    fn extract_caller_paths_includes_the_cli() {
        let all = super::extract_caller_paths();
        let cli: Vec<_> = all.iter().filter(|c| c.source == "cli:amux").collect();
        let spa: Vec<_> = all.iter().filter(|c| c.source == "spa:app.js").collect();
        assert!(!spa.is_empty(), "the SPA scan regressed");
        assert!(
            cli.len() > 10,
            "the CLI scan is not reaching the census — found {} cli callers out of {} total",
            cli.len(),
            all.len()
        );
    }
}
