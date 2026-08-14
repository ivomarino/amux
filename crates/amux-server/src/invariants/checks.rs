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
use std::collections::{BTreeMap, BTreeSet};

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
/// Paths the SPA calls that THIS SERVER NEVER OWNS — the cloud gateway answers
/// them, in front of this process, in the deployment where they exist at all.
///
/// NOT an environment branch (single-codebase rule): these are not served by
/// amux-server in cloud either, so the statement "this server does not own
/// them" is true everywhere and needs no `if IS_CLOUD`. Verified 2026-08-11
/// against cloud/gateway/gateway.py, which handles each one.
///
/// They are excluded because a failure list that can never reach zero stops
/// being read — the same reason the extractor refuses to guess a path. Seven
/// permanent rows would have trained everyone to skim past the real ones.
const GATEWAY_OWNED: &[&str] =
    &["/api/gateway/", "/api/stripe/", "/api/cloud-logout"];

/// An entry ending in `/` is a PREFIX (a whole family); one without is an EXACT
/// path. Applying prefix logic to both over-excluded — `/api/cloud-logout-extra`
/// matched `/api/cloud-logout` and would have been silently dropped from the
/// census. Caught by this function's own test, which is why it asserts the
/// near-misses and not just the hits: an exclusion list that swallows a sibling
/// hides exactly the work it was meant to make visible (ethos rule 1's
/// over-filtering corollary).
fn gateway_owned(path: &str) -> bool {
    GATEWAY_OWNED.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix('/') {
            path == prefix || path.starts_with(p)
        } else {
            path == *p
        }
    })
}

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
        if gateway_owned(&c.path) {
            continue;
        }
        let mut verdict = if c.interpolated {
            match_prefix(mounted, &c.method, &c.path)
        } else {
            match_route_full(mounted, &c.method, &c.path)
        };
        // The verb was DEFAULTED, not observed: `const url = API + '/api/x';
        // ... fetch(url, {method:'POST'})` puts the literal outside the URL's
        // own statement. Asserting the default would file a 405 against a call
        // that never makes it — which is what `GET /api/dictate` was, while the
        // real call is a POST five lines down. Path existence is still checked;
        // only the method claim is withheld.
        if !c.method_known {
            if let RouteMatch::MethodNotAllowed(_) = verdict {
                verdict = RouteMatch::Ok;
            }
        }
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
    /// False when no method literal was found in the call's own statement, so
    /// `method` is the GET DEFAULT rather than something observed. A guessed
    /// verb produces a phantom 405 exactly the way a guessed path produces a
    /// phantom 404 — see the extractor's own note about not guessing paths.
    pub method_known: bool,
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
/// NO TWO LANES MAY SHARE A CLAUDE CONVERSATION (AMUX-1730 / AMUX-2819).
///
/// Two sessions pointed at one `cc_conversation_id` both RESUME it, so a message
/// steered to one surfaces in the other, and work done by one is attributed to
/// the other. It is not theoretical: on 2026-08-10 a fleet scan found two such
/// pairs among 101 lanes —
///     f035d084…  mixpeek-general + mixpeek-frustrations   (BOTH RUNNING)
///     a2f88163…  ts-gke + ts-troubleshooting
/// and the only reason anyone noticed is that a pane title rendered the wrong
/// worker's name. Nothing else reported it.
///
/// The WRITE path is already guarded — `conversation_owned_by_other` gates the
/// single writer of `cc_conversation_id` and both adoption sites — so this
/// check is not redundant with it: the guard prevents NEW cross-links and is
/// blind to the ones already on disk, which is the whole reason these two
/// survived. A guard that cannot see existing damage needs a detector beside
/// it, not a stronger version of itself.
///
/// Pure over (session, conversation) pairs so the real specimen is the test
/// corpus rather than a fixture.
pub fn conversations_are_not_shared(pairs: &[(String, String)]) -> Vec<InvariantResult> {
    const ID: &str = "conversation.one_lane_each";
    let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (session, conv) in pairs {
        if conv.trim().is_empty() {
            continue; // a lane with no conversation yet cannot collide
        }
        by.entry(conv.as_str()).or_default().push(session.as_str());
    }
    let mut out = Vec::new();
    for (conv, mut lanes) in by {
        lanes.sort();
        let short: String = conv.chars().take(8).collect();
        if lanes.len() == 1 {
            out.push(InvariantResult::pass(ID).entity(&short));
        } else {
            out.push(
                InvariantResult::fail(
                    ID,
                    format!("conversation {short} is held by exactly 1 lane"),
                    format!("held by {}: {}", lanes.len(), lanes.join(", ")),
                )
                .entity(&short),
            );
        }
    }
    out
}

/// A card's reviewer must not be the lane that owns it (AMUX-2563).
///
/// The card asked for the SOPHISTICATED version of this — stamp assignments
/// with a conversation id and refuse a review authored from the same transcript
/// — on the theory that two differently-named lanes could share one
/// conversation. Measured 2026-08-11: 99 conversation ids in use, ZERO shared,
/// so that hazard has no live instances (`conversations_are_not_shared` above
/// is what keeps it that way).
///
/// The COARSE version does: 4 live cards carry `reviewer == session`. A lane
/// listed as its own reviewer is self-review by name alone, and no conversation
/// id is needed to see it. Building the fine-grained guard while this went
/// unchecked would have been a guard on the case that does not happen, next to
/// an open door on the case that does.
///
/// Reports rather than refuses: existing assignments belong to whoever made
/// them (ethos rule 8), and a check that surfaces four cards is what lets a
/// human decide, where a retroactive sweep would decide for them.
pub fn reviewer_is_independent(cards: &[(String, String, String)]) -> Vec<InvariantResult> {
    const ID: &str = "board.reviewer_is_independent";
    let mut out = Vec::new();
    for (id, session, reviewer) in cards {
        let (s, r) = (session.trim(), reviewer.trim());
        if r.is_empty() {
            continue; // no reviewer assigned — nothing to be independent of
        }
        if s.is_empty() {
            continue; // unowned card; independence is undefined, not violated
        }
        if s.eq_ignore_ascii_case(r) {
            out.push(
                InvariantResult::fail(
                    ID,
                    format!("{id}: reviewer differs from the owning lane"),
                    format!("both are {s} — the lane would be reviewing its own work"),
                )
                .entity(id),
            );
        } else {
            out.push(InvariantResult::pass(ID).entity(id));
        }
    }
    out
}

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
        if age <= stale_after_s {
            // Recently queued: a normal delivery tick has not elapsed yet.
            out.push(InvariantResult::pass(ID).entity(&it.target));
            continue;
        }
        match it.block_reason.as_deref() {
            // The target is not a live consumer at all (no env file, not
            // running, archived). The message is UNROUTABLE, not merely late:
            // no delivery tick will ever land it, so this is a distinct, louder
            // fact than an idle consumer with lagging delivery, and it was
            // misread as the latter because the invariant did not consult
            // lane_block_reason (AMUX-3084 / AMUX-3111, ethos rule 4: the
            // instrument could not express the discriminator). The cure is a
            // dead-letter path (AMUX-3110), not waiting for a consumer that will
            // never exist.
            Some(reason) => {
                out.push(
                    InvariantResult::fail(
                        ID,
                        format!("queued item delivered or dead-lettered within {stale_after_s:.0}s"),
                        format!("undelivered for {age:.0}s; target is UNROUTABLE ({reason})"),
                    )
                    .entity(&it.target)
                    .evidence(json!({
                        "target": it.target, "age_s": age, "queue": it.queue,
                        "class": "unroutable-target",
                        "block_reason": reason,
                        "fix": "dead-letter unreachable rows (AMUX-3110); do not wait for a \
                                consumer that will never exist",
                    })),
                );
            }
            // A live consumer sitting IDLE with an old item in front of it is
            // the original producer-without-consumer incident: it is not draining.
            None if it.target_idle => {
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
            }
            // A deep queue behind a BUSY worker (routable, not idle) is correct.
            None => out.push(InvariantResult::pass(ID).entity(&it.target)),
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
    /// Why the target is not a deliverable consumer right now, taken from the
    /// SHARED delivery predicate `lane_block_reason` (`no-env-file` /
    /// `not-running` / `archived`), or `None` when the target is a live lane.
    /// Without it the check could not tell an unroutable ghost from an
    /// idle-but-lagging consumer (AMUX-3084 / AMUX-3111).
    pub block_reason: Option<String>,
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

/// The SHARPER contradiction `status_agrees_with_pane` deliberately declines to
/// flag. That check won't call `active` over a quiet pane a fault, because a
/// lane can be legitimately mid-turn with nothing painting (a long tool call, a
/// subagent). But when the harness ITSELF freshly reported `idle` — the main
/// turn stopped — and the pane is not generating, a derived `active` is not
/// "mid-turn with nothing painting": it is amux OVERRIDING the authoritative
/// self-report, the exact inversion of the D1 rule that a fresh report wins.
///
/// INCIDENT (AMUX-3047, 2026-08-13, Ethan "says working but it appears done"):
/// the subagent contradiction in `derive_status` flipped idle->active off a 240s
/// subagent-mtime window with NO report-age gate, so a lane whose stop-hook had
/// posted `idle` ~30s earlier read WORKING for up to four minutes. The root fix
/// gates that flip on report age like the pane contradictions already were; THIS
/// is the log-signal that makes the next instance of the class — any path that
/// derives `active` while a fresh idle self-report AND a non-generating pane both
/// say otherwise — self-announce in /api/health/invariants, instead of waiting
/// for a human to notice a stale badge (the two-fixes rule).
pub fn status_contradicts_fresh_idle_report(lanes: &[LaneTruth]) -> Vec<InvariantResult> {
    const ID: &str = "status.contradicts_fresh_idle_report";
    // Same window as the derivation's `contradiction_window` (D4: policy in
    // config, not baked in). A report younger than this is the authority; a
    // derived `active` over it, with a quiet pane, means the report was
    // overridden.
    let window = std::env::var("AMUX_IDLE_CONTRADICTION_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60.0);
    let mut out = Vec::new();
    for l in lanes {
        let fresh_idle = l.report_state == "idle" && l.report_age_s < window;
        if l.status == "active" && fresh_idle && !l.pane_says_working {
            out.push(
                InvariantResult::fail(
                    ID,
                    "a lane with a fresh idle self-report and a quiet pane is derived active",
                    format!(
                        "status=active while the harness reported idle {:.0}s ago \
                         (< {:.0}s window, source={}) and the pane is not generating",
                        l.report_age_s, window, l.report_source
                    ),
                )
                .entity(&l.name)
                .evidence(json!({
                    "session": l.name,
                    "derived_status": l.status,
                    "report_state": l.report_state,
                    "report_age_s": l.report_age_s,
                    "report_source": l.report_source,
                    "report_origin": l.report_origin,
                    "pane_says_working": l.pane_says_working,
                    "window_s": window,
                    "class": "derived-status-overrides-fresh-self-report",
                    "incident": "AMUX-3047: the subagent contradiction flipped \
                                 idle->active off a 240s mtime window with no \
                                 report-age gate, so a stopped lane read WORKING \
                                 for up to four minutes",
                })),
            );
        } else {
            out.push(InvariantResult::pass(ID).entity(&l.name));
        }
    }
    if out.is_empty() {
        // Same reasoning as status_agrees_with_pane: no lane painted inside the
        // probe window is a real answer on a quiet fleet, not a broken probe.
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
// 5. The report control plane is UP: self-reports are landing at all.
// ---------------------------------------------------------------------------

/// INCIDENT (2026-08-13): the owner reported worker status "inaccurate/delayed"
/// fleet-wide. Root cause: `endpoint.json.legacy_port` went `null` when the 8822
/// bind was dropped, so the Stop/PostToolUse/UserPromptSubmit report hooks baked
/// into ~48 pre-cutover lanes stopped rewriting their stale inherited `AMUX_URL`
/// and POSTed every state report to the dead port — silently (`>/dev/null 2>&1;
/// exit 0`). Measured: 0 of 48 running lanes had a fresh self-report; status
/// fell back entirely to terminal scraping (the D1 path the report endpoint
/// exists to demote). NOTHING in amux surfaced it — the human noticed the
/// symptom, which is exactly the failure the two-fixes rule forbids.
///
/// INVARIANT: on a fleet of any real size, SOMEONE is always at a turn boundary,
/// so the FRESHEST self-report across all running lanes is minutes old, not
/// hours. The discriminator is the FLEET MINIMUM, not any per-lane age: an idle
/// lane legitimately reports once on Stop and then goes quiet for hours (the
/// derivation's asymmetric-freshness rule), so a single stale lane proves
/// nothing — but the youngest report across the WHOLE fleet being hours old
/// means the report control plane is down for everyone at once.
///
/// Gated on `>= min_lanes` running lanes so a one- or two-lane box, where a
/// genuine quiet spell is plausible, reads `Unknown` rather than crying wolf.
pub fn self_reports_landing(
    lanes: &[LaneReport],
    min_lanes: usize,
    max_freshest_s: f64,
) -> Vec<InvariantResult> {
    const ID: &str = "session.self_reports_landing";
    if lanes.len() < min_lanes {
        return vec![InvariantResult::unknown(
            ID,
            format!(
                "only {} running lane(s) (< {min_lanes}) — too few to distinguish a dead \
                 report hook from a genuinely quiet fleet",
                lanes.len()
            ),
        )];
    }
    // Youngest report across the whole fleet, and who it belongs to. A lane with
    // NO report at all contributes nothing to the minimum (it cannot lower it),
    // which is correct: one never-reporting lane is not the fleet-wide outage
    // this catches — a dark fleet minimum is.
    let mut freshest = f64::INFINITY;
    let mut freshest_name = String::new();
    let mut with_report = 0usize;
    for l in lanes {
        if let Some(age) = l.report_age_s {
            with_report += 1;
            if age < freshest {
                freshest = age;
                freshest_name = l.name.clone();
            }
        }
    }
    if with_report == 0 {
        // Not one running lane has EVER reported: the control plane is fully
        // down, not merely quiet.
        return vec![InvariantResult::fail(
            ID,
            format!("at least one of {} running lanes reporting", lanes.len()),
            format!(
                "0 of {} running lanes have any self-report — report hooks are not landing",
                lanes.len()
            ),
        )
        .evidence(json!({
            "running_lanes": lanes.len(),
            "lanes_with_report": 0,
            "class": "report-control-plane-down",
            "incident": "2026-08-13: endpoint.json.legacy_port went null; baked-in report \
                         hooks POSTed to the dead 8822 and failed silently",
            "likely_cause": "endpoint.json legacy_port/retired_ports not naming the port \
                             pre-cutover sessions carry — status is running blind on pane-scrape",
        }))];
    }
    if freshest > max_freshest_s {
        return vec![InvariantResult::fail(
            ID,
            format!("freshest self-report across the fleet < {max_freshest_s:.0}s"),
            format!(
                "youngest report across {} running lanes is {freshest:.0}s old (from {freshest_name}; \
                 {with_report} lanes carry any report) — report control plane down fleet-wide, \
                 status is on pane-scrape",
                lanes.len()
            ),
        )
        .evidence(json!({
            "running_lanes": lanes.len(),
            "lanes_with_report": with_report,
            "freshest_report_age_s": freshest,
            "freshest_lane": freshest_name,
            "threshold_s": max_freshest_s,
            "class": "report-control-plane-down",
            "incident": "2026-08-13: baked-in report hooks POSTed to the dead 8822 silently; \
                         0/48 fresh self-reports, worker status inaccurate/delayed",
        }))];
    }
    vec![InvariantResult::pass(ID).evidence(json!({
        "running_lanes": lanes.len(),
        "lanes_with_report": with_report,
        "freshest_report_age_s": freshest,
        "freshest_lane": freshest_name,
    }))]
}

/// One running lane's self-report age, `None` when the lane has never reported.
#[derive(Debug, Clone)]
pub struct LaneReport {
    pub name: String,
    pub report_age_s: Option<f64>,
}

// ---------------------------------------------------------------------------
// Negative controls (AMUX-2624). Each proves the check DETECTS the real bug.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod negative_controls {
    use super::*;
    use crate::invariants::Status;

    /// CORPUS IS THE LIVE FLEET on 2026-08-10, not a fixture: two shared
    /// conversations among 101 lanes, one of them held by two RUNNING lanes.
    #[test]
    fn two_lanes_on_one_conversation_is_a_failure_naming_both() {
        let pairs: Vec<(String, String)> = vec![
            ("mixpeek-general".into(), "f035d084-b362-404f-8cd3-d5ae76d17c28".into()),
            ("mixpeek-frustrations".into(), "f035d084-b362-404f-8cd3-d5ae76d17c28".into()),
            ("ts-gke".into(), "a2f88163-1111-2222-3333-444444444444".into()),
            ("ts-troubleshooting".into(), "a2f88163-1111-2222-3333-444444444444".into()),
            ("amux".into(), "1dd2cd21-c4a7-46b9-9b97-51fccbe721a2".into()),
        ];
        let rs = conversations_are_not_shared(&pairs);
        let fails: Vec<&InvariantResult> = rs.iter().filter(|r| r.status != Status::Pass).collect();
        assert_eq!(fails.len(), 2, "both shared conversations must fail: {rs:?}");
        // BOTH lane names must appear in the observed value. "conversation
        // f035d084 is shared" without them sends the reader to the meta files to
        // work out who — which is the hand-search that found this originally.
        let obs: String = fails.iter().map(|f| f.observed.clone()).collect::<Vec<_>>().join(" ");
        for lane in ["mixpeek-general", "mixpeek-frustrations", "ts-gke", "ts-troubleshooting"] {
            assert!(obs.contains(lane), "{lane} missing from the failure: {obs}");
        }
        // The healthy lane passes — a check that fails for everyone is not a check.
        assert_eq!(rs.iter().filter(|r| r.status == Status::Pass).count(), 1);
    }

    /// A lane with no conversation yet cannot collide, and must not be reported
    /// as sharing the empty string with every other new lane — which is what a
    /// naive group-by does, turning a fresh fleet into one giant failure.
    #[test]
    fn lanes_without_a_conversation_are_not_a_collision() {
        let pairs: Vec<(String, String)> = vec![
            ("a".into(), "".into()),
            ("b".into(), "".into()),
            ("c".into(), "   ".into()),
        ];
        assert!(conversations_are_not_shared(&pairs).is_empty());
    }

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
            interpolated: false, method_known: true,
        }];
        let rs = route_callers_have_routes(&mounted(), &callers);
        assert!(
            rs.iter().any(|r| r.status == Status::Fail),
            "the census MUST fail on the /api/workers/<n>/send gap — this is the \
             bug the spec names as the thing it should have caught"
        );
    }

    /// Gateway-owned paths are excluded, and ONLY those. A list that can never
    /// reach zero stops being read, but over-excluding hides real misses — so
    /// this pins both directions.
    #[test]
    fn only_gateway_owned_paths_are_excluded() {
        assert!(gateway_owned("/api/gateway/orgs"));
        assert!(gateway_owned("/api/stripe/checkout"));
        assert!(gateway_owned("/api/cloud-logout"));
        // Near-misses that this server DOES own must still be checked.
        assert!(!gateway_owned("/api/gatewayish"), "prefix must not swallow a sibling");
        assert!(!gateway_owned("/api/board"));
        assert!(!gateway_owned("/api/sql"));
        assert!(!gateway_owned("/api/cloud-logout-extra"), "only the exact logout path");
    }

    #[test]
    fn detects_a_lane_listed_as_its_own_reviewer() {
        let cards = vec![
            ("A-1".into(), "amux".into(), "amux".into()),          // violation
            ("A-2".into(), "amux".into(), "AMUX".into()),          // same, case-folded
            ("A-3".into(), "amux".into(), "creative-dna".into()),  // fine
            ("A-4".into(), "amux".into(), "".into()),              // no reviewer: skipped
            ("A-5".into(), "".into(), "amux".into()),              // unowned: skipped
        ];
        let out = reviewer_is_independent(&cards);
        let failed: Vec<&str> = out
            .iter()
            .filter(|r| r.status != crate::invariants::Status::Pass)
            .map(|r| r.entity_key.as_str())
            .collect();
        assert_eq!(failed, vec!["A-1", "A-2"], "self-review, including case-folded");
        assert_eq!(out.len(), 3, "cards with no reviewer or no owner are not judged");
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
            interpolated: false, method_known: true,
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
            block_reason: None,
        }];
        let rs = queue_has_live_consumer(&items, 7_560.0, 300.0); // 2h6m, the real age
        assert!(rs.iter().any(|r| r.status == Status::Fail), "must detect the dead consumer");
    }

    /// AMUX-3084 / AMUX-3111: a target that is not a live consumer at all (its
    /// env file is gone after the amux-rust->amux rename) must read as
    /// UNROUTABLE, not as an idle consumer with lagging delivery. Before this the
    /// invariant branched only on target_idle and reported the ghost as
    /// producer-without-consumer, sending the reader to "wait for the consumer"
    /// when the truth was "this consumer will never exist".
    #[test]
    fn a_ghost_target_reads_as_unroutable_not_a_dead_consumer() {
        let items = vec![QueuedItem {
            queue: "steering".into(),
            target: "amux-rust".into(),
            queued_at: 0.0,
            target_idle: true, // carries a stale, never-decaying idle report (AMUX-2646)
            block_reason: Some("no-env-file".into()),
        }];
        let rs = queue_has_live_consumer(&items, 7_560.0, 300.0);
        let f = rs
            .iter()
            .find(|r| r.status == Status::Fail)
            .expect("an 18h-stuck row must still fail");
        assert_eq!(
            f.evidence["class"].as_str(),
            Some("unroutable-target"),
            "a ghost target must be classed unroutable, not producer-without-consumer: {}",
            f.evidence
        );
        assert!(
            f.observed.contains("UNROUTABLE"),
            "the observed sentence must name the routability fault: {}",
            f.observed
        );
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

    /// AMUX-3047, rebuilt from the incident artifact: gtm-engine derived
    /// `active` while its stop-hook had posted `idle` 30s earlier (inside the
    /// 60s window) and the pane was a quiet "✻ Crunched for 1m 7s" prompt. The
    /// log-signal must catch this class — a fresh self-report overridden.
    #[test]
    fn fresh_idle_report_contradiction_fires_on_active_over_fresh_idle() {
        let lanes = vec![LaneTruth {
            name: "gtm-engine".into(),
            status: "active".into(),
            pane_says_working: false,
            report_state: "idle".into(),
            report_age_s: 30.0,
            report_source: "stop-hook".into(),
            report_origin: "gtm-engine".into(),
        }];
        let rs = status_contradicts_fresh_idle_report(&lanes);
        assert!(
            rs.iter().any(|r| r.status == Status::Fail),
            "must flag active derived over a fresh idle self-report + quiet pane"
        );
        assert_eq!(rs[0].entity_key, "gtm-engine", "the failure must name the lane");
    }

    /// Must NOT fire once the idle report ages past the window: a still-writing
    /// subagent flipping it active then is the bounded late correction, not a
    /// bug. A check that fires on legitimate behaviour gets switched off.
    #[test]
    fn fresh_idle_report_contradiction_silent_on_a_stale_report() {
        let lanes = vec![LaneTruth {
            name: "gtm-engine".into(),
            status: "active".into(),
            pane_says_working: false,
            report_state: "idle".into(),
            report_age_s: 120.0, // past the 60s window
            report_source: "stop-hook".into(),
            report_origin: "gtm-engine".into(),
        }];
        assert!(status_contradicts_fresh_idle_report(&lanes)
            .iter()
            .all(|r| r.status == Status::Pass));
    }

    /// Must NOT fire when the pane genuinely IS generating — then `active` is
    /// correct regardless of any report, and this is not an override.
    #[test]
    fn fresh_idle_report_contradiction_silent_when_pane_is_working() {
        let lanes = vec![LaneTruth {
            name: "gtm-engine".into(),
            status: "active".into(),
            pane_says_working: true,
            report_state: "idle".into(),
            report_age_s: 30.0,
            report_source: "stop-hook".into(),
            report_origin: "gtm-engine".into(),
        }];
        assert!(status_contradicts_fresh_idle_report(&lanes)
            .iter()
            .all(|r| r.status == Status::Pass));
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
            block_reason: None,
        }];
        let rs = queue_has_live_consumer(&items, 7_560.0, 300.0);
        assert!(
            rs.iter().all(|r| r.status == Status::Pass),
            "a deep queue behind a busy worker is correct, not a fault"
        );
    }

    // -- session.self_reports_landing (the 2026-08-13 reporting outage) --------

    /// THE INCIDENT'S OWN ARTIFACT: the 2026-08-13 fleet, freshest report from
    /// `primis` at 7379s and everything else 40h+. The check must FAIL and name
    /// the freshest lane and its age — the fleet MINIMUM is what discriminates a
    /// dead control plane from a legitimately quiet lane.
    #[test]
    fn a_fleet_whose_youngest_report_is_hours_old_fails() {
        // Ages drawn from the real outage: one 2h outlier, the rest ~40h.
        let mut lanes: Vec<LaneReport> = (0..47)
            .map(|i| LaneReport {
                name: format!("lane-{i}"),
                report_age_s: Some(143_000.0 + i as f64),
            })
            .collect();
        lanes.push(LaneReport { name: "primis".into(), report_age_s: Some(7_379.0) });
        let rs = self_reports_landing(&lanes, 10, 3600.0);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].status, Status::Fail, "youngest 7379s > 3600s must fail: {rs:?}");
        // Names the freshest lane and age, so the reader does not re-derive it.
        assert!(rs[0].observed.contains("primis"), "must name freshest lane: {}", rs[0].observed);
        assert!(rs[0].observed.contains("7379"), "must state the age: {}", rs[0].observed);
    }

    /// A healthy fleet: someone reported seconds ago, so the minimum is fresh
    /// even though most lanes are idle-and-quiet. Must PASS — a check that fires
    /// on the normal steady state gets ignored, and then it is not a check.
    #[test]
    fn a_fleet_with_one_fresh_report_passes_even_if_most_are_stale() {
        let mut lanes: Vec<LaneReport> = (0..40)
            .map(|i| LaneReport {
                name: format!("idle-{i}"),
                report_age_s: Some(30_000.0),
            })
            .collect();
        lanes.push(LaneReport { name: "busy".into(), report_age_s: Some(4.0) });
        let rs = self_reports_landing(&lanes, 10, 3600.0);
        assert!(
            rs.iter().all(|r| r.status == Status::Pass),
            "a fresh fleet minimum is healthy even behind idle lanes: {rs:?}"
        );
    }

    /// Not one lane has ever reported: the control plane is fully down — a
    /// distinct, louder failure than a merely-stale minimum.
    #[test]
    fn a_fleet_with_zero_reports_fails_as_control_plane_down() {
        let lanes: Vec<LaneReport> = (0..20)
            .map(|i| LaneReport { name: format!("l-{i}"), report_age_s: None })
            .collect();
        let rs = self_reports_landing(&lanes, 10, 3600.0);
        assert_eq!(rs[0].status, Status::Fail);
        assert!(rs[0].observed.contains("0 of 20"), "{}", rs[0].observed);
    }

    /// A one- or two-lane box must read Unknown, never fire: a genuine quiet
    /// spell is plausible there, and a false alarm trains the reader to skim.
    #[test]
    fn a_tiny_fleet_is_unknown_not_a_false_alarm() {
        let lanes = vec![LaneReport { name: "solo".into(), report_age_s: Some(999_999.0) }];
        let rs = self_reports_landing(&lanes, 10, 3600.0);
        assert_eq!(rs[0].status, Status::Unknown, "too-small fleet must be Unknown: {rs:?}");
    }
}
