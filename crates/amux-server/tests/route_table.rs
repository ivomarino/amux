//! ROUTE_TABLE honesty test (AMUX-2610).
//!
//! `request_log::ROUTE_TABLE` is the hand-maintained routing truth behind
//! `GET /api/debug/routes` and the 404/405 annotations + verdicts in
//! `GET /api/logs/analyze`. axum's Router cannot enumerate its routes, so
//! the table can only stay honest by being WALKED against the real
//! composition (`api::router`), and the walk must be able to fail BOTH
//! directions:
//!
//! - **Claimed but not routed**: an OPTIONS probe at a table path that axum
//!   does not route lands in the SPA catch-all — a nested miss answers 404,
//!   a top-level miss answers the catch-all's 405 whose `Allow` is exactly
//!   `GET(,HEAD)`. Concrete entries assert 405 with `Allow` EQUAL (as a
//!   set, HEAD/OPTIONS aside) to the table's methods, so a bogus path fails
//!   either on status or on the Allow set; the one ambiguous case — a
//!   GET-only entry, whose Allow matches the GET-only catch-all's — is
//!   disambiguated by firing the GET and rejecting the catch-all's
//!   BYTE-EXACT 404 body (`{"error": "not found"}` WITH the space —
//!   hand-written only in static_files.rs; every module body is serde-
//!   serialized without it).
//! - **Routed but not claimed** (an under-listed method): the `Allow` set
//!   equality catches it, and the negative twin — a method the table does
//!   NOT list — is fired and must answer 405. If the router actually
//!   mounts that method, the handler answers something else and the walk
//!   fails, forcing the table to list it.
//!
//! Both directions were demonstrated against deliberately-wrong entries
//! before this landed (a nonexistent path, and a GET-only claim on
//! /api/board which also routes POST); see AMUX-2610.
//!
//! `["*"]` (any()) entries invoke their handler on OPTIONS, so for them the
//! walk asserts the answer is NOT the catch-all's signature (405 with
//! Allow ⊆ {GET, HEAD}, or an HTML 404).

use amux_server::api::request_log::ROUTE_TABLE;
use amux_server::api::{router, AppState};
use amux_server::db::Store;
use axum::body::Body;
use axum::http::Request;
use std::collections::BTreeSet;
use tower::ServiceExt;

/// The SPA catch-all's 404 body for /api paths, byte-exact (the SPACE is the
/// discriminator — see module doc).
const STATIC_404_BODY: &str = "{\"error\": \"not found\"}";

fn concretize(pattern: &str) -> String {
    pattern
        .split('/')
        .map(|seg| {
            if seg.starts_with("{*") {
                "zz-probe"
            } else if seg.starts_with('{') {
                "zz-probe-1"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn allow_set(res: &axum::response::Response) -> BTreeSet<String> {
    res.headers()
        .get("allow")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(',')
        .map(|m| m.trim().to_uppercase())
        .filter(|m| !m.is_empty() && m != "HEAD" && m != "OPTIONS")
        .collect()
}

async fn fire(app: &axum::Router, method: &str, path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

/// One test fn on purpose: it mutates process env (AMUX_PY_URL, AMUX_HOME)
/// and tests within a binary share the process (same shape as
/// proxy_composition.rs).
#[tokio::test]
async fn route_table_matches_the_real_router_both_directions() {
    // Dead python: /api/scope's any() probe must answer a deterministic 502,
    // never touch a real server.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    std::env::set_var("AMUX_PY_URL", &dead);
    // Hermetic fleet home: probes on sessions/groups/files must not read the
    // developer's real ~/.amux.
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("sessions")).unwrap();
    std::env::set_var("AMUX_HOME", home.path());

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    let app = router(AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: None,
    });

    let twin_candidates = ["DELETE", "PUT", "PATCH", "POST", "GET"];
    for entry in ROUTE_TABLE {
        let path = concretize(entry.path);
        let res = fire(&app, "OPTIONS", &path).await;
        let status = res.status().as_u16();

        if entry.methods.contains(&"*") {
            // any(): OPTIONS reaches the handler. The only failure shape is
            // the catch-all answering instead of a route.
            let allow = allow_set(&res);
            let catchall_405 =
                status == 405 && !allow.is_empty() && allow.iter().all(|m| m == "GET");
            assert!(
                !catchall_405,
                "{}: claimed any() but OPTIONS hit the GET-only SPA catch-all (405, allow={allow:?})",
                entry.path
            );
            let ct = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert!(
                !(status == 404 && ct.starts_with("text/html")),
                "{}: claimed any() but answered the SPA shell",
                entry.path
            );
            continue;
        }

        // Concrete methods: the route's MethodRouter must answer the OPTIONS
        // probe with 405 + an Allow set equal to the table's claim.
        assert_eq!(
            status, 405,
            "{}: not routed where the table claims (OPTIONS answered {status}, not the \
             method-router's 405)",
            entry.path
        );
        let allow = allow_set(&res);
        let claimed: BTreeSet<String> =
            entry.methods.iter().map(|m| m.to_uppercase()).collect();
        assert_eq!(
            allow, claimed,
            "{}: table methods disagree with the router's Allow set",
            entry.path
        );

        // GET-only ambiguity: the catch-all is itself a GET route at every
        // path, so Allow == {GET} does not prove THIS route exists. The GET
        // answer does: the catch-all's /api 404 body is byte-exact.
        if claimed.len() == 1 && claimed.contains("GET") {
            let res = fire(&app, "GET", &path).await;
            let status = res.status().as_u16();
            // Read the body ONLY on 404: a non-404 already proves a real
            // route answered, and some GET routes stream forever
            // (/api/events is SSE — to_bytes on it never returns).
            if status == 404 {
                let body =
                    axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
                let body = String::from_utf8_lossy(&body);
                assert!(
                    body != STATIC_404_BODY,
                    "{}: GET answered the SPA catch-all's 404 — the table claims a \
                     route that does not exist",
                    entry.path
                );
            }
        }

        // Negative twin: a method the table does NOT list must 405 (an
        // under-listed method would reach a handler and answer otherwise).
        let twin = twin_candidates
            .iter()
            .find(|m| !claimed.contains(**m))
            .expect("a method outside the claimed set always exists");
        let res = fire(&app, twin, &path).await;
        assert_eq!(
            res.status().as_u16(),
            405,
            "{}: unlisted method {twin} did not 405 — the router mounts more than the \
             table lists",
            entry.path
        );
    }

    std::env::remove_var("AMUX_PY_URL");
    std::env::remove_var("AMUX_HOME");
}
