//! Every diagnostic endpoint says whether its measurement RAN (AF-320).
//!
//! `frustrations.md` carries 41 `AREA: instruments` entries out of 83, plus 24
//! more in the archive. No other area is close. The shape is the same every
//! time: the measurement returns a clean value, the value is wrong, and nothing
//! beside it says the probe could not have produced a right one.
//!
//! - AMUX-3772 — the latency card named an innocent endpoint, confidently backwards.
//! - AMUX-2644 — a worker whose pane died at launch reports `running: true` / `idle`.
//! - AR-111   — the request log recorded a ~15s restart choreography as a 76ms request.
//! - AEAB-42  — the disk ranker cannot rank a file, so it could never have named the 1.8GB one.
//! - AMUX-2841 — a probe read a hook file git never executes; a correct
//!   measurement certified the wrong conclusion.
//!
//! Ethos rule 4 already states the remedy: any output that can read zero or
//! empty must publish, in the same payload, whether the measurement ran. That
//! is a rule which asks people to remember, and half the ledger is people not
//! remembering. This test is the mechanical half.
//!
//! It enumerates the diagnostic families out of `ROUTE_TABLE` rather than a
//! list kept here, so a NEW diagnostic route is in scope the moment it is
//! mounted. That is the property worth having: a hand-maintained list in a test
//! only covers the endpoints someone remembered to add, which is the failure
//! mode being fixed.

use amux_server::api::request_log::ROUTE_TABLE;
use amux_server::api::{router, AppState};
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt;

/// A path is a diagnostic if it belongs to one of the instrument families.
///
/// Family prefixes, not an enumeration of endpoints: `/api/debug/x` added
/// tomorrow is covered without anyone editing this file.
fn is_diagnostic(path: &str) -> bool {
    path.starts_with("/api/debug/") || path == "/api/logs" || path.starts_with("/api/logs/") || path == "/api/health/invariants"
}

/// Diagnostic GET routes with no path parameter — the ones a caller can hit
/// with no arguments and get a report back.
///
/// Parameterised paths (`/api/debug/x/{id}`) are excluded because there is no
/// argument-free way to call them here, not because they are exempt from the
/// contract. If one ever appears this comment is the place to fix it, and the
/// count assertion below is what makes it visible.
fn diagnostic_paths() -> Vec<&'static str> {
    ROUTE_TABLE
        .iter()
        .filter(|r| is_diagnostic(r.path))
        .filter(|r| !r.path.contains('{'))
        .filter(|r| r.methods.contains(&"GET") || r.methods.contains(&"*"))
        .map(|r| r.path)
        .collect()
}

fn app() -> axum::Router {
    let dir = tempfile::tempdir().unwrap();
    // Leaked on purpose: the store must outlive the router for the duration of
    // the test process, and a temp dir dropped here would delete the db under it.
    let dir = Box::leak(Box::new(dir));
    let store = amux_server::db::Store::open(&dir.path().join("diag.db")).unwrap();
    router(AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        // None disables auth, so protected diagnostics answer here too.
        auth_token: None,
    reconciled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    })
}

async fn get_json(path: &str) -> (u16, Option<Value>) {
    let res = app()
        .oneshot(Request::builder().method("GET").uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let body = axum::body::to_bytes(res.into_body(), 32 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).ok())
}

/// THE WINDOW PARAMETER IS `since_h` ACROSS THE FAMILY, and one probe did not
/// speak it (AF-482).
///
/// `/api/debug/duplicate-deliveries` read `hours` alone. `since_h` is what
/// CLAUDE.md's observability table documents and what /api/logs/analyze,
/// /api/logs/stats, /api/logs/writers, /api/debug/sse, the git-guard and the
/// messages probes all take, so a caller reaching for the convention got the
/// 24h default with no sign of it. Measured 2026-09-04: since_h=24, 72, 168 and
/// 720 all returned `total: 7` while 26 duplicate events existed.
///
/// THE CHECK IS THAT THE WINDOW MOVES, not that a parameter name appears. A
/// probe that echoes `hours` and ignores it would pass a name check and is
/// exactly the bug. Two calls, two very different windows, and the reported
/// window must differ; a probe that reports no window at all is out of scope
/// rather than failing, because most diagnostics have none.
///
/// Enumerated from ROUTE_TABLE like the cell below it, so a new diagnostic that
/// invents a third spelling is caught the day it mounts.
#[tokio::test]
async fn a_diagnostic_that_reports_a_window_honours_since_h() {
    let paths = diagnostic_paths();
    assert!(
        paths.len() >= 15,
        "the ROUTE_TABLE walk found only {} routes; a green result would mean nothing",
        paths.len()
    );

    // The field a probe uses to report the window it actually used. `hours` is
    // included deliberately: the offender echoed it truthfully while ignoring
    // the request, and a true field that reads as agreement is the trap.
    fn reported_window(v: &Value) -> Option<f64> {
        for k in ["since_h", "hours", "window_h", "actual_window_h"] {
            if let Some(n) = v.get(k).and_then(|x| x.as_f64()) {
                return Some(n);
            }
        }
        None
    }

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for p in &paths {
        let (_s1, narrow) = get_json(&format!("{p}?since_h=1")).await;
        let (_s2, wide) = get_json(&format!("{p}?since_h=500")).await;
        let (Some(a), Some(b)) = (narrow, wide) else { continue };
        let (Some(wa), Some(wb)) = (reported_window(&a), reported_window(&b)) else {
            // Reports no window. Nothing to honour, nothing to check.
            continue;
        };
        checked += 1;
        if (wa - wb).abs() < f64::EPSILON {
            offenders.push(format!(
                "{p} — reports a window but since_h=1 and since_h=500 both yield {wa}"
            ));
        }
    }

    // POSITIVE CONTROL, and it is the whole reason this cell is not vacuous: if
    // no diagnostic reports a window, every probe is skipped and the loop above
    // asserts nothing. The offender itself is one of them, so this must hold.
    assert!(
        checked >= 2,
        "only {checked} diagnostic(s) reported a window at all, so this cell tested          almost nothing. Either the extractor regressed or the window fields were renamed."
    );
    assert!(
        offenders.is_empty(),
        "a diagnostic reports a window it does not take from `since_h`, so a caller \n         following the fleet convention silently gets the default (AF-482):\n  {}",
        offenders.join("\n  ")
    );
}

/// THE CHECK. Every argument-free diagnostic GET publishes `measured` and
/// `n_considered`.
#[tokio::test]
async fn every_diagnostic_says_whether_its_measurement_ran() {
    let paths = diagnostic_paths();

    // POSITIVE CONTROL. If the extractor stops finding routes this test would
    // pass by measuring nothing, which is the exact defect it is about.
    assert!(
        paths.len() >= 15,
        "the ROUTE_TABLE walk found only {} diagnostic routes — the extractor regressed, \
         and a green result here would mean nothing.\nfound: {paths:?}",
        paths.len()
    );
    assert!(
        paths.contains(&"/api/logs/analyze"),
        "the specimen endpoint is missing from the walk: {paths:?}"
    );

    let mut missing: Vec<String> = Vec::new();
    for p in &paths {
        let (status, json) = get_json(p).await;
        let Some(v) = json else {
            missing.push(format!("{p} — answered {status} with no JSON body"));
            continue;
        };
        let measured = v.get("measured");
        let n = v.get("n_considered");
        match (measured, n) {
            (Some(Value::Bool(_)), Some(n)) if n.is_u64() => {}
            (m, n) => missing.push(format!(
                "{p} — measured={} n_considered={}",
                m.map_or("ABSENT".into(), |m| m.to_string()),
                n.map_or("ABSENT".into(), |n| n.to_string()),
            )),
        }
    }

    assert!(
        missing.is_empty(),
        "{} diagnostic endpoint(s) return a count, verdict or ranking without saying whether \
         the measurement ran. A bare zero and an unmeasurable zero read identically, which is \
         41 of 83 frustration entries (ethos rule 4).\n  {}\n\nStamp them with \
         `api::measured::measured(body, n)` or `api::measured::unmeasured(body, why)`.",
        missing.len(),
        missing.join("\n  ")
    );
}

/// `measured: false` must be REACHABLE, or the contract is a constant.
///
/// A field that is always `true` carries no information and would pass the test
/// above forever. This asserts the unmeasured arm is a real branch by exercising
/// the helper itself: the endpoints cannot be forced into their failure state
/// from here, but the shape they must produce when they get there can be.
#[test]
fn the_unmeasured_arm_is_a_real_shape_not_a_constant() {
    let v = amux_server::api::measured::unmeasured(
        serde_json::json!({"groups": [], "total_errors": 0}),
        "the log file does not exist yet",
    );
    assert_eq!(v["measured"], Value::Bool(false));
    assert_eq!(v["n_considered"], serde_json::json!(0));
    assert_eq!(v["why_unmeasured"], serde_json::json!("the log file does not exist yet"));
    // The report's own fields survive the stamp — a wrapper that replaced the
    // body would make every caller's zero unreadable in a different way.
    assert_eq!(v["total_errors"], serde_json::json!(0));

    let m = amux_server::api::measured::measured(serde_json::json!({"total_errors": 0}), 4210);
    assert_eq!(m["measured"], Value::Bool(true));
    assert_eq!(m["n_considered"], serde_json::json!(4210));
    // A measured report does NOT carry why_unmeasured — the two arms are
    // distinguishable by shape, not only by value.
    assert!(m.get("why_unmeasured").is_none(), "{m}");
}
