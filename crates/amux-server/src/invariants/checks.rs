//! The invariant checks themselves (AMUX-2622).
//!
//! EVERY check in here is derived from an incident that actually happened in
//! this repo, and each one tests the INVARIANT the incident revealed rather
//! than the implementation detail that broke (spec §29). The incident is named
//! in the doc comment so the next person can tell whether a "simplification"
//! would re-open it.
//!
//! Each check ships with a negative control at the bottom of this file: a test
//! that INJECTS the failure and asserts the check reports it. Per AMUX-2624, a
//! check that has never been demonstrated failing is not a valid health check —
//! this repo has shipped a green `if True:` fixture, a grep that could not
//! match, and a spin-catcher that ranked sleeping threads, all of which "passed".

use super::InvariantResult;
use serde_json::json;
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------
// 1. Route contract: every path a CLIENT calls must be mounted.
// ---------------------------------------------------------------------------

/// INCIDENT: `POST /api/workers/<n>/send` returned 405 after the Python
/// retirement while `/api/sessions/<n>/send` returned 200. The installed CLI
/// posts to the canonical spelling, so `amux send` degraded fleet-wide to raw
/// tmux keystroke injection — unstamped, unaudited, delivery unverified — and
/// two long inter-session messages were lost before a human noticed.
///
/// INVARIANT: a path that a shipped client (SPA or CLI) calls must resolve to a
/// mounted route. This is the check the spec names explicitly: "this should
/// have caught the /api/workers/<name>/send 405 before production".
///
/// Deliberately compares against the ROUTER'S OWN TABLE rather than a
/// hand-written expectation list, and normalises `{param}` segments, so adding
/// a caller without a route fails even if nobody remembers to update a fixture.
pub fn route_callers_have_routes(
    mounted: &[(&str, &[&str])],
    callers: &[CallerPath],
) -> Vec<InvariantResult> {
    const ID: &str = "route.callers_have_routes";
    if callers.is_empty() {
        // An extractor that found nothing is broken, not vindicated. This is
        // the empty-grep trap: a probe that could not match reports the same
        // silence as a system with no problems.
        return vec![InvariantResult::unknown(
            ID,
            "no client call sites extracted — the extractor is broken, not the fleet clean",
        )];
    }
    let mut out = Vec::new();
    for c in callers {
        let verdict = if c.interpolated {
            match_prefix(mounted, &c.method, &c.path)
        } else {
            match_route_full(mounted, &c.method, &c.path)
        };
        match verdict {
            RouteMatch::Missing => out.push(
                InvariantResult::fail(
                    ID,
                    format!("{} {} is mounted", c.method, c.path),
                    "no route matches this path".to_string(),
                )
                .entity(format!("{} {}", c.method, c.path))
                .evidence(json!({
                    "caller": c.source, "method": c.method, "path": c.path,
                    "class": "route-missing",
                    "why_it_matters": "the client calls this; a 404/405 here is a silent \
                                       capability loss unless the client fails loudly",
                })),
            ),
            RouteMatch::MethodNotAllowed(allowed) => out.push(
                InvariantResult::fail(
                    ID,
                    format!("{} allowed on {}", c.method, c.path),
                    format!("route exists but allows only {allowed:?} — {} would 405", c.method),
                )
                .entity(format!("{} {}", c.method, c.path))
                .evidence(json!({
                    "caller": c.source, "method": c.method, "path": c.path,
                    "allowed": allowed, "class": "verb-missing",
                    "incident": "amux send -> /api/workers/<n>/send 405 -> raw tmux fallback",
                })),
            ),
            RouteMatch::Ok => out.push(InvariantResult::pass(ID).entity(format!("{} {}", c.method, c.path))),
        }
    }
    out
}

/// A path a shipped client actually calls.
#[derive(Debug, Clone)]
pub struct CallerPath {
    pub method: String,
    pub path: String,
    /// Where it was found — "spa:app.js" / "cli:amux". Carried so a failure
    /// names the file to fix rather than just the path.
    pub source: String,
    /// The literal was followed by concatenation/interpolation, so `path` is a
    /// PREFIX, not the whole request path (`'/api/board/' + id`).
    ///
    /// This distinction is the difference between a usable check and an ignored
    /// one. Treating a prefix as an exact path produced 86 false failures on
    /// the first live run — every `/api/board/<id>` DELETE reported as "DELETE
    /// not allowed on /api/board" — which is precisely the cry-wolf outcome the
    /// module docs warn about. A prefix is satisfied when SOME mounted route
    /// lives under it with the right method.
    pub interpolated: bool,
}

#[derive(Debug, PartialEq)]
enum RouteMatch {
    Ok,
    MethodNotAllowed(Vec<String>),
    Missing,
}

/// Segment-wise pattern match, with axum's semantics: `{name}` matches exactly
/// one segment, `{*rest}` matches the remainder.
///
/// Segment-wise and NOT substring, deliberately. A prefix matcher would report
/// `/api/workers/x/send` as covered by `/api/workers` — a false pass that would
/// let this entire check exist and still miss the incident it was built for.
/// `a_prefix_does_not_count_as_a_match` pins that.
fn segments_match(pat: &[&str], want: &[&str]) -> bool {
    let mut i = 0;
    while i < pat.len() {
        let p = pat[i];
        if p.starts_with("{*") {
            // wildcard tail: must have at least one segment left to consume
            return want.len() > i;
        }
        if i >= want.len() {
            return false;
        }
        if p.starts_with('{') {
            i += 1;
            continue; // one-segment param
        }
        if p != want[i] {
            return false;
        }
        i += 1;
    }
    pat.len() == want.len()
}

/// Resolve a concrete (method, path) against the mounted table.
///
/// Distinguishes Missing from MethodNotAllowed because the two have different
/// fixes — mount the route vs add the verb — and the incident that motivated
/// this check was the SECOND kind, which a boolean "is it routed" would have
/// called healthy.
/// Resolve an INTERPOLATED caller prefix: `'/api/board/' + id` can only be
/// checked as "does some route live under /api/board with this method".
///
/// Weaker than the exact match on purpose, and the weakness is the point: an
/// exact check on a prefix is not a stricter check, it is a WRONG one, and its
/// failures are noise that gets the whole monitor ignored. Exact literals still
/// go through `match_route_full`, so the /api/workers/<n>/send class — a fully
/// literal path in the CLI — keeps its precision.
fn match_prefix(mounted: &[(&str, &[&str])], method: &str, prefix: &str) -> RouteMatch {
    let want: Vec<&str> = prefix.trim_matches('/').split('/').collect();
    let mut method_seen: Option<Vec<String>> = None;
    for (pat, methods) in mounted {
        let pv: Vec<&str> = pat.trim_matches('/').split('/').collect();
        // The mounted route must be AT or BELOW the prefix: every literal
        // segment of the prefix has to line up with the pattern.
        if pv.len() < want.len() {
            continue;
        }
        let aligned = want.iter().enumerate().all(|(i, w)| {
            let p = pv[i];
            p.starts_with('{') || p == *w
        });
        if !aligned {
            continue;
        }
        if methods.contains(&"*") || methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
            return RouteMatch::Ok;
        }
        method_seen = Some(methods.iter().map(|s| s.to_string()).collect());
    }
    match method_seen {
        Some(a) => RouteMatch::MethodNotAllowed(a),
        None => RouteMatch::Missing,
    }
}

fn match_route_full(mounted: &[(&str, &[&str])], method: &str, path: &str) -> RouteMatch {
    let want: Vec<&str> = path.trim_matches('/').split('/').collect();
    let mut allowed_seen: Option<Vec<String>> = None;
    for (pat, methods) in mounted {
        let pv: Vec<&str> = pat.trim_matches('/').split('/').collect();
        if !segments_match(&pv, &want) {
            continue;
        }
        if methods.contains(&"*") || methods.iter().any(|m| m.eq_ignore_ascii_case(method)) {
            return RouteMatch::Ok;
        }
        allowed_seen = Some(methods.iter().map(|s| s.to_string()).collect());
    }
    match allowed_seen {
        Some(a) => RouteMatch::MethodNotAllowed(a),
        None => RouteMatch::Missing,
    }
}

// ---------------------------------------------------------------------------
// 2. Config provenance: a configured value must reach the process.
// ---------------------------------------------------------------------------

/// INCIDENT: `~/.amux/server.env` held flags that never reached
/// `std::env::var`, so every consumer read the default and the configuration
/// was silently dead ("server.env actually setdefaults into the process env —
/// flags read via std::env::var were silently dead").
///
/// INVARIANT: for every key in server.env, the process env agrees. Spec §14:
/// "this would have caught values existing in server.env but not reaching
/// std::env::var".
///
/// Values are never emitted — only key names and an agree/differ verdict — so
/// this is safe to expose on a health endpoint. server.env is the one place
/// credential VALUES live.
pub fn config_env_reaches_process(env_file: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<InvariantResult> {
    const ID: &str = "config.env_reaches_process";
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for line in env_file.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim();
        if k.is_empty() || !seen.insert(k.to_string()) {
            continue;
        }
        // Quotes are stripped because a value read straight out of the file
        // with its quotes attached is its own documented incident in this repo
        // (an `[ -d ]` test reported an existing directory as missing).
        let want = v.trim().trim_matches('"').trim_matches('\'');
        match lookup(k) {
            Some(got) if got == want => out.push(InvariantResult::pass(ID).entity(k)),
            Some(_) => out.push(
                InvariantResult::fail(ID, format!("{k} = (server.env value)"), format!("{k} = (different process value)"))
                    .entity(k)
                    .evidence(json!({
                        "key": k, "class": "config-drift",
                        "note": "values intentionally omitted — server.env holds credentials",
                    })),
            ),
            None => out.push(
                InvariantResult::fail(ID, format!("{k} present in process env"), format!("{k} unset in process env"))
                    .entity(k)
                    .evidence(json!({
                        "key": k, "class": "config-not-reaching-process",
                        "incident": "server.env flags read via std::env::var were silently dead",
                    })),
            ),
        }
    }
    if out.is_empty() {
        return vec![InvariantResult::unknown(ID, "server.env unreadable or empty")];
    }
    out
}

// ---------------------------------------------------------------------------
// 3. Queue liveness: a producer must have a consumer.
// ---------------------------------------------------------------------------

/// INCIDENT (twice, same week): the steering queue had three producers and NO
/// consumer — messages were stored durably and never delivered, so a lane sat
/// IDLE with 9 QUEUED, the oldest 2h6m old. Separately, auto-pickup died with
/// the Python retirement and 6 idle lanes sat on 17 dispatchable cards.
///
/// INVARIANT: a queued item must be progressing. If the oldest undelivered item
/// is older than `stale_after_s` while its target is IDLE, the consumer is not
/// running — which is a different, louder fact than "the queue is deep".
///
/// The IDLE qualifier is load-bearing: a deep queue behind a busy worker is
/// correct behaviour, and flagging it would train everyone to ignore this.
pub fn queue_has_live_consumer(items: &[QueuedItem], now: f64, stale_after_s: f64) -> Vec<InvariantResult> {
    const ID: &str = "queue.has_live_consumer";
    let mut out = Vec::new();
    for it in items {
        let age = now - it.queued_at;
        // Only an item whose target is idle proves the consumer is absent.
        if it.target_idle && age > stale_after_s {
            out.push(
                InvariantResult::fail(
                    ID,
                    format!("queued item delivered within {stale_after_s:.0}s of the target going idle"),
                    format!("undelivered for {age:.0}s while target is IDLE"),
                )
                .entity(&it.target)
                .evidence(json!({
                    "target": it.target, "age_s": age, "queue": it.queue,
                    "class": "producer-without-consumer",
                    "incident": "steering queue had 3 producers and no consumer; auto-pickup \
                                 died with the python retirement",
                })),
            );
        } else {
            out.push(InvariantResult::pass(ID).entity(&it.target));
        }
    }
    if out.is_empty() {
        out.push(InvariantResult::pass(ID));
    }
    out
}

#[derive(Debug, Clone)]
pub struct QueuedItem {
    pub queue: String,
    pub target: String,
    pub queued_at: f64,
    pub target_idle: bool,
}

// ---------------------------------------------------------------------------
// 4. Status truth: the card must agree with the pane.
// ---------------------------------------------------------------------------

/// INCIDENT (AMUX-2646): `amux-rust` showed `idle` on its card while its pane
/// read `esc to interrupt`. Its self-report was a fabricated
/// `{"state":"idle","source":"stop-hook-test"}` written by a hand-run hook
/// test onto a live lane, and the derivation's asymmetric freshness rule says
/// an `idle` report never decays — so nothing in the system could disagree
/// with it. A human spotted it by looking at a terminal.
///
/// INVARIANT: a lane whose pane is unambiguously mid-turn is not reported
/// `idle`. Two sources of truth — the derived card status and the physical
/// pane — and this is the seam between them, which is precisely where no
/// component health check ever looks: the report store was healthy, the
/// derivation was healthy, the pane was healthy, and they disagreed.
///
/// This is the check that would have caught it in seconds. It is cheap enough
/// to run on the monitor tick because the caller only probes lanes that
/// painted recently — a lane that has not painted cannot be mid-turn.
pub fn status_agrees_with_pane(lanes: &[LaneTruth]) -> Vec<InvariantResult> {
    const ID: &str = "status.agrees_with_pane";
    let mut out = Vec::new();
    for l in lanes {
        // Only ONE direction is a contradiction. A card reading `active` over
        // a quiet pane is not: a lane can be legitimately mid-turn with
        // nothing painting (a long tool call, a subagent), and flagging it
        // would fire constantly and train everyone to ignore this.
        if l.pane_says_working && l.status == "idle" {
            out.push(
                InvariantResult::fail(
                    ID,
                    "a lane whose pane is mid-turn is not reported idle",
                    format!(
                        "card={} while the pane shows work (report={} age={:.0}s source={})",
                        l.status, l.report_state, l.report_age_s, l.report_source
                    ),
                )
                .entity(&l.name)
                .evidence(json!({
                    "session": l.name,
                    "card_status": l.status,
                    "pane_says_working": true,
                    "report_state": l.report_state,
                    "report_age_s": l.report_age_s,
                    "report_source": l.report_source,
                    "report_origin": l.report_origin,
                    "class": "report-outranks-physical-evidence",
                    "incident": "AMUX-2646: a hand-run hook test wrote idle onto a live \
                                 working lane; an idle report never decays, so nothing \
                                 could contradict it",
                })),
            );
        } else {
            out.push(InvariantResult::pass(ID).entity(&l.name));
        }
    }
    if out.is_empty() {
        // No lane painted inside the probe window. That is a real answer on a
        // quiet fleet, not a broken probe: the caller only enumerates lanes it
        // could actually read.
        out.push(InvariantResult::pass(ID));
    }
    out
}

/// One lane's two sources of truth, side by side.
#[derive(Debug, Clone)]
pub struct LaneTruth {
    pub name: String,
    /// What the card says (the derived status).
    pub status: String,
    /// What the pane says — computed with the SAME detectors the derivation
    /// uses, so the check and the mechanism cannot disagree about what
    /// "working" means.
    pub pane_says_working: bool,
    pub report_state: String,
    pub report_age_s: f64,
    pub report_source: String,
    pub report_origin: String,
}

// ---------------------------------------------------------------------------
// Negative controls (AMUX-2624). Each proves the check DETECTS the real bug.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod negative_controls {
    use super::*;
    use crate::invariants::Status;

    fn mounted() -> Vec<(&'static str, &'static [&'static str])> {
        vec![
            ("/api/sessions/{name}/{*verb}", &["*"][..]),
            ("/api/board", &["GET", "POST"][..]),
            ("/api/workers", &["GET"][..]),
        ]
    }

    /// NEGATIVE CONTROL for the exact production incident: the CLI calls
    /// /api/workers/<n>/send, only the /api/sessions spelling is mounted.
    /// Pre-fix this is what production looked like, and the check must FAIL.
    #[test]
    fn detects_the_workers_send_405_that_shipped() {
        let callers = vec![CallerPath {
            method: "POST".into(),
            path: "/api/workers/amux/send".into(),
            source: "cli:amux".into(),
            interpolated: false,
        }];
        let rs = route_callers_have_routes(&mounted(), &callers);
        assert!(
            rs.iter().any(|r| r.status == Status::Fail),
            "the census MUST fail on the /api/workers/<n>/send gap — this is the \
             bug the spec names as the thing it should have caught"
        );
    }

    /// ...and must PASS once the canonical spelling is mounted, or it is a
    /// check that always fires, which is the same as no check.
    #[test]
    fn passes_once_the_canonical_route_is_mounted() {
        let mut m = mounted();
        m.push(("/api/workers/{name}/{*verb}", &["*"][..]));
        let callers = vec![CallerPath {
            method: "POST".into(),
            path: "/api/workers/amux/send".into(),
            source: "cli:amux".into(),
            interpolated: false,
        }];
        let rs = route_callers_have_routes(&m, &callers);
        assert!(rs.iter().all(|r| r.status == Status::Pass), "must pass after the fix");
    }

    /// A route that exists but lacks the VERB is the 405 case specifically, and
    /// must be reported differently from "missing" — the two have different
    /// fixes (mount vs add method).
    #[test]
    fn distinguishes_verb_missing_from_route_missing() {
        assert_eq!(
            match_route_full(&mounted(), "DELETE", "/api/board"),
            RouteMatch::MethodNotAllowed(vec!["GET".into(), "POST".into()])
        );
        assert_eq!(match_route_full(&mounted(), "GET", "/api/nope"), RouteMatch::Missing);
        assert_eq!(match_route_full(&mounted(), "POST", "/api/board"), RouteMatch::Ok);
    }

    /// THE FALSE-PASS GUARD. A substring/prefix matcher would call
    /// /api/workers/x/send "covered" by /api/workers and report health — the
    /// exact way this check could exist and still miss the incident.
    #[test]
    fn a_prefix_does_not_count_as_a_match() {
        assert_eq!(
            match_route_full(&mounted(), "POST", "/api/workers/amux/send"),
            RouteMatch::Missing,
            "/api/workers must NOT satisfy /api/workers/amux/send"
        );
    }

    /// An extractor that found nothing must report Unknown, never a clean pass.
    /// The empty-grep trap: silence from a broken probe is indistinguishable
    /// from silence from a healthy system unless it is typed differently.
    #[test]
    fn no_callers_extracted_is_unknown_not_pass() {
        let rs = route_callers_have_routes(&mounted(), &[]);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].status, Status::Unknown, "empty extraction is a broken probe");
    }

    /// NEGATIVE CONTROL: server.env key that never reached the process.
    #[test]
    fn detects_config_that_never_reached_the_process() {
        let envf = "AMUX_RS_SCHEDULER=true\nAMUX_OK=1\n";
        let rs = config_env_reaches_process(envf, &|k| match k {
            "AMUX_OK" => Some("1".into()),
            _ => None, // AMUX_RS_SCHEDULER never made it — the real incident
        });
        let failed: Vec<_> = rs.iter().filter(|r| r.status == Status::Fail).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].entity_key, "AMUX_RS_SCHEDULER");
    }

    /// Quoted values must not be reported as drift — a value read with its
    /// quotes still attached is its own incident in this repo.
    #[test]
    fn quoted_values_are_not_false_drift() {
        let rs = config_env_reaches_process("K=\"v\"\n", &|_| Some("v".into()));
        assert!(rs.iter().all(|r| r.status == Status::Pass), "quotes must be stripped before comparing");
    }

    /// NEGATIVE CONTROL: the producer-without-consumer shape. An old item in
    /// front of an IDLE target is proof the consumer is not running.
    #[test]
    fn detects_a_queue_whose_consumer_is_dead() {
        let items = vec![QueuedItem {
            queue: "steering".into(),
            target: "amux-rust".into(),
            queued_at: 0.0,
            target_idle: true,
        }];
        let rs = queue_has_live_consumer(&items, 7_560.0, 300.0); // 2h6m, the real age
        assert!(rs.iter().any(|r| r.status == Status::Fail), "must detect the dead consumer");
    }

    /// NEGATIVE CONTROL, rebuilt from the incident's own artifact: the exact
    /// row `amux-rust` had on 2026-08-09 — a card reading `idle` behind a
    /// 1076-second-old `stop-hook-test` report, over a pane that was mid-turn.
    #[test]
    fn detects_the_card_that_said_idle_while_the_pane_was_working() {
        let lanes = vec![LaneTruth {
            name: "amux-rust".into(),
            status: "idle".into(),
            pane_says_working: true,
            report_state: "idle".into(),
            report_age_s: 1076.0,
            report_source: "stop-hook-test".into(),
            report_origin: String::new(),
        }];
        let rs = status_agrees_with_pane(&lanes);
        assert!(
            rs.iter().any(|r| r.status == Status::Fail),
            "must detect a card that contradicts its own pane"
        );
        assert_eq!(rs[0].entity_key, "amux-rust", "the failure must name the lane");
    }

    /// ...and must NOT fire in the other direction. A lane reported `active`
    /// with a quiet pane is a long tool call or a subagent, which is normal —
    /// a check that fires on normal operation is one people switch off.
    #[test]
    fn does_not_fire_on_an_active_card_over_a_quiet_pane() {
        let lanes = vec![LaneTruth {
            name: "amux".into(),
            status: "active".into(),
            pane_says_working: false,
            report_state: "active".into(),
            report_age_s: 4.0,
            report_source: "tool-hook".into(),
            report_origin: "amux".into(),
        }];
        assert!(status_agrees_with_pane(&lanes).iter().all(|r| r.status == Status::Pass));
    }

    /// The agreeing case must PASS rather than being unrepresentable — a check
    /// whose only outcome is failure cannot tell health from silence.
    #[test]
    fn passes_when_the_card_and_the_pane_agree() {
        let lanes = vec![LaneTruth {
            name: "amux".into(),
            status: "active".into(),
            pane_says_working: true,
            report_state: "active".into(),
            report_age_s: 2.0,
            report_source: "tool-hook".into(),
            report_origin: "amux".into(),
        }];
        assert!(status_agrees_with_pane(&lanes).iter().all(|r| r.status == Status::Pass));
    }

    /// ...and must NOT fire for a deep queue behind a BUSY worker, which is
    /// correct behaviour. A check that flags normal operation gets ignored, and
    /// then it is not a check.
    #[test]
    fn does_not_fire_for_a_queue_behind_a_busy_worker() {
        let items = vec![QueuedItem {
            queue: "steering".into(),
            target: "amux-rust".into(),
            queued_at: 0.0,
            target_idle: false, // mid-turn: queueing is the POINT
        }];
        let rs = queue_has_live_consumer(&items, 7_560.0, 300.0);
        assert!(
            rs.iter().all(|r| r.status == Status::Pass),
            "a deep queue behind a busy worker is correct, not a fault"
        );
    }
}
