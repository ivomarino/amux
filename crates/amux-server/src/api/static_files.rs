//! Embedded dashboard serving (RR-0021 + Phase 8 bootstrap injection).
//!
//! Files come from amux-dashboard's `static/` at compile time. index.html
//! gets its AMUX-BOOTSTRAP block substituted at serve time — the same
//! values the Python server injects (amux-server.py:65679), same trust
//! model: the dashboard shell + auth token are served unauthenticated on
//! the LAN, exactly as the Python server does today. Cloud deployments put
//! a gateway in front of both. Parity, not a new decision.

use super::AppState;
use amux_dashboard::DashboardAssets;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use sha2::Digest;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(index))
        .route("/{*path}", axum::routing::get(serve_path))
}

async fn index(State(state): State<AppState>) -> Response {
    serve_index(&state)
}

async fn serve_path(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // UNKNOWN /api/* paths reach this catch-all (the API router only claims
    // registered routes) and must answer the Python server's JSON 404 — not
    // the SPA shell. Serving 200 text/html here made a probe conclude
    // "GET /api/fs?path=/tmp returns 200 with NO token": the "endpoint" was
    // this fallback handing back index.html (ethos rule 4 — the instrument
    // could not express "no such route").
    if path.starts_with("api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\": \"not found\"}",
        )
            .into_response();
    }
    match DashboardAssets::get(path) {
        Some(content) => {
            let mime = mime_for(path);
            ([(header::CONTENT_TYPE, mime)], content.data.into_owned()).into_response()
        }
        // SPA fallback: unknown NON-API paths get the shell so client routing
        // works offline-first.
        None => serve_index(&state),
    }
}

fn serve_index(state: &AppState) -> Response {
    let Some(index) = DashboardAssets::get("index.html") else {
        return (StatusCode::NOT_FOUND, "dashboard not embedded").into_response();
    };
    let html = String::from_utf8_lossy(&index.data).into_owned();
    let injected = inject_bootstrap(&html, state);
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        injected,
    )
        .into_response()
}

/// Replace the marked bootstrap block with live values. The UI token is
/// derived exactly as the Python does (sha256("amux-ui-guard:"+AUTH)[..40],
/// amux-server.py:801) so a dashboard served by either server produces
/// headers the OTHER server also accepts during coexistence.
fn inject_bootstrap(html: &str, state: &AppState) -> String {
    const BEGIN: &str = "<!-- AMUX-BOOTSTRAP-BEGIN";
    const END: &str = "<!-- AMUX-BOOTSTRAP-END -->";
    let (Some(b), Some(e)) = (html.find(BEGIN), html.find(END)) else {
        return html.to_string(); // no markers: serve untouched, never corrupt
    };
    let auth = state.auth_token.clone().unwrap_or_default();
    let ui_token = if auth.is_empty() {
        String::new()
    } else {
        let mut h = sha2::Sha256::new();
        h.update(format!("amux-ui-guard:{auth}"));
        hex::encode(h.finalize())[..40].to_string()
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let jstr = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let block = format!(
        "<!-- AMUX-BOOTSTRAP-BEGIN (injected at serve time) -->\n<script>\
         window._AMUX_S3_ICAL_URL={};window._AMUX_AUTH_TOKEN={};window._AMUX_HOME={};\
         window._AMUX_POSTHOG_KEY={};window._AMUX_POSTHOG_HOST={};window._AMUX_USER_EMAIL={};\
         window._AMUX_USER_ID={};window._AMUX_UI_TOKEN={};window._AMUX_DEFAULT_MODEL={};</script>\n",
        jstr(&std::env::var("AMUX_S3_ICAL_URL").unwrap_or_default()),
        jstr(&auth),
        jstr(&home),
        jstr(&std::env::var("POSTHOG_KEY").unwrap_or_default()),
        jstr(&std::env::var("POSTHOG_HOST").unwrap_or_else(|_| "https://us.i.posthog.com".into())),
        jstr(&std::env::var("AMUX_USER_EMAIL").unwrap_or_default()),
        jstr(&std::env::var("AMUX_USER_ID").unwrap_or_default()),
        jstr(&ui_token),
        // The REAL configured default, not a hardcoded guess — the settings
        // sweep caught the select showing sonnet after a PATCH (finding #2).
        jstr(&crate::api::settings::get_default_model(
            &std::env::var("AMUX_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
                        .join(".amux")
                }),
        )),
    );
    let with_bootstrap = format!("{}{}{}", &html[..b], block, &html[e..]);
    // Update watcher (serve-time layer, never touching the extracted SPA):
    // polls /health and, when the server's build hash moves — the builder
    // installed a new backend — offers a RELOAD the user chooses to take.
    // Backend changes flow live regardless (SSE reconnects on its own and
    // re-pushes full state); this banner is only about adopting new CLIENT
    // code, and that adoption is the user's call, never forced.
    // CRM is removed from the Rust build (Ethan, 2026-08-09): hide its tab
    // and view via the serve-time layer — the extracted SPA stays
    // byte-identical, the decision lives HERE where it is one greppable
    // line to reverse.
    let crm_hide = r#"<style>/* AMUX-FEATURE-FLAGS (injected) */
[onclick="switchView('crm')"], #crm-view { display: none !important; }
</style>
"#;
    let watcher = format!(
        r#"<script>/* AMUX-UPDATE-WATCH (injected at serve time) */
(function() {{
  var atLoad = {build:?};
  function check() {{
    fetch('/health').then(function(r) {{ return r.json(); }}).then(function(h) {{
      if (h && h.build && h.build !== atLoad && !document.getElementById('amux-update-bar')) {{
        var bar = document.createElement('div');
        bar.id = 'amux-update-bar';
        bar.style.cssText = 'position:fixed;bottom:calc(12px + env(safe-area-inset-bottom));left:50%;transform:translateX(-50%);z-index:9998;background:#1f6feb;color:#fff;padding:10px 14px;border-radius:10px;font:600 0.85rem -apple-system,system-ui,sans-serif;display:flex;gap:12px;align-items:center;box-shadow:0 4px 16px rgba(0,0,0,0.4);';
        bar.innerHTML = 'Server updated (' + h.build.slice(0,8) + ') <button onclick="location.reload()" style="min-width:44px;min-height:32px;background:#fff;color:#1f6feb;border:0;border-radius:8px;font-weight:700;cursor:pointer;padding:6px 12px;">Reload</button><button onclick="this.parentNode.remove()" style="min-width:32px;min-height:32px;background:transparent;color:#fff;border:0;font-size:1rem;cursor:pointer;">&#215;</button>';
        document.body.appendChild(bar);
      }}
    }}).catch(function() {{}});
  }}
  setInterval(check, 30000);
}})();
</script>
"#,
        build = state.build_hash,
    );
    with_bootstrap.replacen("</body>", &format!("{crm_hide}{watcher}</body>"), 1)
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("webmanifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    fn state(token: Option<&str>) -> AppState {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::db::Store::open(&dir.path().join("t.db")).unwrap());
        std::mem::forget(dir);
        AppState {
            store,
            started: Instant::now(),
            build_hash: "test".into(),
            auth_token: token.map(String::from),
        }
    }

    #[test]
    fn bootstrap_injects_auth_and_derived_ui_token() {
        let html = "<head><!-- AMUX-BOOTSTRAP-BEGIN x -->old<!-- AMUX-BOOTSTRAP-END --></head>";
        let out = inject_bootstrap(html, &state(Some("tok123")));
        assert!(out.contains("window._AMUX_AUTH_TOKEN=\"tok123\""));
        // Python-parity UI token: sha256("amux-ui-guard:tok123")[..40]
        let mut h = sha2::Sha256::new();
        h.update("amux-ui-guard:tok123");
        let expect = &hex::encode(h.finalize())[..40];
        assert!(out.contains(expect), "{out}");
        assert!(!out.contains("old"), "placeholder block replaced");
    }

    #[test]
    fn update_watcher_carries_the_serving_build() {
        let html = "<head><!-- AMUX-BOOTSTRAP-BEGIN x -->old<!-- AMUX-BOOTSTRAP-END --></head><body></body>";
        let s = state(Some("tok"));
        let out = inject_bootstrap(html, &s);
        assert!(out.contains("AMUX-UPDATE-WATCH"));
        assert!(out.contains(&format!("var atLoad = {:?}", s.build_hash)));
        // The banner is offered, never forced: reload only behind a click.
        assert!(out.contains(">Reload</button>"));
        assert!(!out.contains("location.reload();</script>"), "no unconditional reload");
    }

    #[test]
    fn missing_markers_serve_untouched() {
        let html = "<head>no markers</head>";
        assert_eq!(inject_bootstrap(html, &state(None)), html);
    }

    #[tokio::test]
    async fn unknown_api_path_is_a_json_404_not_the_spa_shell() {
        use tower::ServiceExt;
        let app = routes().with_state(state(Some("tok")));
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/definitely-not-a-route?x=1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v, serde_json::json!({ "error": "not found" }));

        // Non-API unknown paths still get the SPA shell (client routing).
        let res = routes()
            .with_state(state(Some("tok")))
            .oneshot(
                axum::http::Request::builder()
                    .uri("/some/client/route")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct.starts_with("text/html"), "{ct}");
    }
}
