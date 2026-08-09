//! FULL-composition routing tests for the python-proxy passthroughs
//! (AMUX-2594 + the /api/fs swallow).
//!
//! Why these exist at the integration level and not only as unit tests: the
//! nested routers' unit tests build `Router::new().nest(...)` WITHOUT the
//! static SPA catch-all (`/{*path}`), and in the full `api::router`
//! composition that catch-all out-competes a nested `.fallback()` — so a
//! fallback-based proxy passed its unit test while the live server answered
//! index.html (200 text/html) for /api/groups, /api/browser/state and every
//! other unrouted API path. That 200-HTML swallow is what broke the SPA's
//! group picker ("adding a group didn't work": app.js fetches /api/groups,
//! r.json() threw on HTML, the .catch silently emptied the dropdown) and
//! what earlier misled an auth probe into reporting an unauthenticated 200
//! on /api/fs. These tests pin the property that failed: proxied namespaces
//! must never be answered by the static shell.
//!
//! The Python base URL points at a dead port on purpose: the assertion is
//! about ROUTING (request reaches the proxy handler, which then answers an
//! honest 502 naming the unreachable Python), deterministic on any machine —
//! a live-Python assertion here would pass on the dev box and rot in CI.

use amux_server::api::{router, AppState};
use amux_server::db::Store;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("amux-test.db")).unwrap();
    let state = AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: None,
    };
    (router(state), dir)
}

async fn get(app: &axum::Router, path: &str) -> (StatusCode, String, String) {
    let res = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, ct, String::from_utf8_lossy(&bytes).into_owned())
}

/// One test fn on purpose: it mutates process env (AMUX_PY_URL), and tests
/// within a binary share the process.
#[tokio::test]
async fn proxied_namespaces_route_to_python_never_to_the_spa_shell() {
    // Dead port: bind-then-drop reserves an address nothing serves.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    std::env::set_var("AMUX_PY_URL", &dead);

    let (app, _dir) = app();

    // Every proxied namespace, including the AMUX-2594 group-picker path and
    // sub-paths that have no explicit Rust route.
    for path in [
        "/api/groups",
        "/api/groups/mygroup/config",
        "/api/tags",
        "/api/tags/mytag",
        "/api/fs",
        "/api/fs/list?path=/tmp",
        "/api/browser/state?session=x",
        "/api/browser/pw-profiles",
        "/api/browser/screenshot",
    ] {
        let (status, ct, body) = get(&app, path).await;
        assert!(
            !ct.starts_with("text/html"),
            "{path} answered by the static shell: {ct}"
        );
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{path}: {body}");
        let v: Value = serde_json::from_slice(body.as_bytes()).unwrap();
        assert_eq!(
            v["error"], "python server unreachable for proxied path",
            "{path}: {body}"
        );
    }

    // Unrouted dictation paths answer the module's NATIVE Python-shape 404
    // (never the static shell's generic one, never a proxy attempt).
    let (status, ct, body) = get(&app, "/api/dictation/bogus").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(ct.starts_with("application/json"), "{ct}");
    let v: Value = serde_json::from_slice(body.as_bytes()).unwrap();
    assert_eq!(v["error"], "dictation route not found", "{body}");

    // Unknown API paths outside every nest: static's JSON 404 (Python's
    // generic shape), NOT the SPA shell.
    let (status, ct, body) = get(&app, "/api/definitely-not-a-route").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(ct.starts_with("application/json"), "{ct}");
    let v: Value = serde_json::from_slice(body.as_bytes()).unwrap();
    assert_eq!(v["error"], "not found", "{body}");

    // Non-API unknown paths still serve the SPA shell (client routing).
    let (status, ct, _body) = get(&app, "/some/client/route").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.starts_with("text/html"), "{ct}");

    std::env::remove_var("AMUX_PY_URL");
}
