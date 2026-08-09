//! GET /api/usage — subscription usage for the Settings meter (Python
//! amux-server.py:71557, `_fetch_claude_usage` ~:3189).
//!
//! The endpoint is wired to the PROVIDER LAYER, not to its own probe: the
//! Claude OAuth usage adapter already exists (provider/claude.rs) and is the
//! one place that talks to api.anthropic.com. This handler only SHAPES the
//! adapter's normalized windows into the Python response the SPA's
//! `loadUsage()` consumes:
//!
//! - success: `{"available": true, "limits": [{kind, percent, resets_at}...]}`
//!   (Python passes Anthropic's raw body through with `available: true`; the
//!   SPA reads exactly `available` / `reason` / `limits[].kind` /
//!   `limits[].percent` / `limits[].resets_at` / `limits[].scope` /
//!   `limits[].group` — grep loadUsage in app.js). The adapter normalizes
//!   away per-model scope, so shaped entries carry kind/percent/resets_at;
//!   `kind` is "session" for the 5-hour window and "weekly" for 7-day
//!   windows, both of which the SPA labels correctly.
//! - degraded: `{"available": false, "reason": "..."}` — the same shape
//!   Python serves for no-token / expired / HTTP failure. The adapter
//!   collapses those causes into "unknown" (Invariant 20), so the reason is
//!   one honest sentence, never invented numbers.
//!
//! The 30-second cache is Python's `_usage_cache` / `_USAGE_TTL` verbatim:
//! reopening the settings panel must not hammer Anthropic. Degraded results
//! are cached too (Python caches whatever `_fetch_claude_usage` returned).

use std::sync::Arc;
use std::time::{Duration, Instant};

use amux_core::provider::{UsageWindow, UsageWindowKind};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json, Router};
use serde_json::{json, Value};

use super::AppState;
use crate::provider::ProviderAdapter;

/// Python `_USAGE_TTL = 30`.
const USAGE_TTL: Duration = Duration::from_secs(30);

/// One honest sentence for every degraded cause — the adapter deliberately
/// collapses no-token / expired / HTTP-error into "unknown" rather than
/// letting this endpoint guess which one happened (a specific-but-wrong
/// reason is worse than a broad true one).
const UNAVAILABLE_REASON: &str =
    "Claude subscription usage unavailable on this host (no token, expired token, or probe failed)";

#[derive(Default)]
struct UsageCache {
    data: Option<Value>,
    at: Option<Instant>,
}

/// Production wiring: the Claude adapter, the same instance shape the
/// provider registry registers.
pub fn routes() -> Router<AppState> {
    routes_with(Arc::new(crate::provider::claude::ClaudeAdapter::new()))
}

/// Test seam: the adapter is injected so tests exercise the exact handler
/// with a mock and never touch the network or the keychain.
pub fn routes_with(adapter: Arc<dyn ProviderAdapter>) -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::get(get_usage))
        .layer(Extension(adapter))
        .layer(Extension(Arc::new(tokio::sync::Mutex::new(UsageCache::default()))))
}

async fn get_usage(
    Extension(adapter): Extension<Arc<dyn ProviderAdapter>>,
    Extension(cache): Extension<Arc<tokio::sync::Mutex<UsageCache>>>,
) -> Response {
    let mut c = cache.lock().await;
    let fresh = matches!((&c.data, c.at), (Some(_), Some(at)) if at.elapsed() <= USAGE_TTL);
    if !fresh {
        let usage = adapter.usage().await;
        c.data = Some(shape_usage(&usage.windows));
        c.at = Some(Instant::now());
    }
    Json(c.data.clone().unwrap_or_else(|| json!({}))).into_response()
}

/// Windows -> the Python wire shape. Zero windows is the adapter's honest
/// "unknown" and maps to Python's degraded `{"available": false, "reason"}`;
/// anything else becomes `{"available": true, "limits": [...]}`.
fn shape_usage(windows: &[UsageWindow]) -> Value {
    let limits: Vec<Value> = windows.iter().filter_map(shape_window).collect();
    if limits.is_empty() {
        return json!({ "available": false, "reason": UNAVAILABLE_REASON });
    }
    json!({ "available": true, "limits": limits })
}

/// One window -> one `limits[]` entry. A window without a reported `used`
/// yields NO entry (nothing reported -> nothing invented); percent is the
/// reported utilization (adapter contract: used = percent, limit = 100),
/// recomputed only when a future adapter reports a real cap.
fn shape_window(w: &UsageWindow) -> Option<Value> {
    let used = w.used?;
    let percent = match w.limit {
        // The adapter's definitional form: used IS the percentage.
        None | Some(100) => used as f64,
        Some(0) => return None, // a 0-cap window has no meaningful percent
        Some(limit) => ((used as f64) * 100.0 / (limit as f64)).round(),
    };
    let kind = match w.window_kind {
        // The SPA orders "session"-kind first and labels weekly* windows;
        // these are the two kinds the Claude adapter produces.
        UsageWindowKind::Rolling => Value::String("session".into()),
        UsageWindowKind::Weekly => Value::String("weekly".into()),
        // Future kinds pass through under their serde name — the SPA falls
        // back to displaying the kind string itself.
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    };
    Some(json!({
        "kind": kind,
        "percent": percent,
        "resets_at": w.resets_at.map(|d| d.to_rfc3339()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use amux_core::provider::{
        ProviderCapabilities, ProviderId, ProviderUsage, UsageConfidence, UsageProvenance,
    };
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// Mock adapter: fixed windows, counts calls. NEVER touches network,
    /// keychain, or credentials (the whole point of routes_with).
    struct MockAdapter {
        windows: Vec<UsageWindow>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ProviderAdapter for MockAdapter {
        fn id(&self) -> ProviderId {
            ProviderId::new("claude-code")
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities::default()
        }
        async fn usage(&self) -> ProviderUsage {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ProviderUsage::new(self.id(), self.windows.clone())
        }
        async fn models(&self) -> Vec<String> {
            Vec::new()
        }
        fn build_command(&self, _mode: crate::provider::PromptMode) -> Vec<String> {
            vec!["mock".into()]
        }
    }

    fn window(kind: UsageWindowKind, pct: u64, resets: Option<&str>) -> UsageWindow {
        UsageWindow {
            window_kind: kind,
            used: Some(pct),
            limit: Some(100),
            resets_at: resets
                .map(|s| chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)),
            confidence: UsageConfidence::Exact,
            provenance: UsageProvenance::Api,
        }
    }

    fn app(adapter: Arc<MockAdapter>) -> axum::Router {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::db::Store::open(&dir.path().join("usage-test.db")).unwrap();
        std::mem::forget(dir);
        let state = AppState {
            store: Arc::new(store),
            started: Instant::now(),
            build_hash: "test".into(),
            auth_token: None,
        };
        Router::new().nest("/api/usage", routes_with(adapter)).with_state(state)
    }

    async fn get(app: &axum::Router) -> (StatusCode, Value) {
        let res = app
            .clone()
            .oneshot(Request::builder().uri("/api/usage").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn reported_windows_serve_pythons_available_shape() {
        let adapter = Arc::new(MockAdapter {
            windows: vec![
                window(UsageWindowKind::Rolling, 34, Some("2026-08-09T18:00:00Z")),
                window(UsageWindowKind::Weekly, 72, Some("2026-08-12T00:00:00Z")),
                window(UsageWindowKind::Weekly, 103, None),
            ],
            calls: AtomicUsize::new(0),
        });
        let app = app(adapter);
        let (st, v) = get(&app).await;
        assert_eq!(st, StatusCode::OK);
        // The keys loadUsage() reads: available + limits[{kind,percent,resets_at}].
        assert_eq!(v["available"], json!(true));
        let limits = v["limits"].as_array().unwrap();
        assert_eq!(limits.len(), 3);
        assert_eq!(limits[0]["kind"], json!("session"));
        assert_eq!(limits[0]["percent"], json!(34.0));
        assert_eq!(limits[0]["resets_at"], json!("2026-08-09T18:00:00+00:00"));
        assert_eq!(limits[1]["kind"], json!("weekly"));
        // Over-limit stays visible (a real state), and a missing reset is
        // null, not a fabricated timestamp.
        assert_eq!(limits[2]["percent"], json!(103.0));
        assert_eq!(limits[2]["resets_at"], Value::Null);
        assert!(v.get("reason").is_none());
    }

    #[tokio::test]
    async fn unknown_usage_serves_pythons_degraded_shape_with_no_numbers() {
        let adapter = Arc::new(MockAdapter { windows: vec![], calls: AtomicUsize::new(0) });
        let app = app(adapter);
        let (st, v) = get(&app).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["available"], json!(false));
        assert!(v["reason"].as_str().unwrap().contains("unavailable"));
        // Degraded means NO limits key at all — never invented entries.
        assert!(v.get("limits").is_none(), "{v}");
    }

    #[tokio::test]
    async fn thirty_second_cache_prevents_hammering_the_provider() {
        let adapter = Arc::new(MockAdapter {
            windows: vec![window(UsageWindowKind::Rolling, 10, None)],
            calls: AtomicUsize::new(0),
        });
        let app = app(adapter.clone());
        let (_, first) = get(&app).await;
        let (_, second) = get(&app).await;
        let (_, third) = get(&app).await;
        assert_eq!(first, second);
        assert_eq!(second, third);
        // Python's _USAGE_TTL contract: repeated opens within 30s = one probe.
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn windowless_and_partial_windows_never_yield_entries() {
        // A window with no reported number produces no limits entry; with
        // ONLY such windows the response is the degraded shape (parity with
        // Python, where a fetch that reports nothing shows nothing).
        let adapter = Arc::new(MockAdapter {
            windows: vec![UsageWindow::unknown(UsageWindowKind::Rolling, UsageProvenance::Api)],
            calls: AtomicUsize::new(0),
        });
        let app = app(adapter);
        let (_, v) = get(&app).await;
        assert_eq!(v["available"], json!(false));
    }

    #[test]
    fn shape_window_recomputes_percent_only_for_real_caps() {
        // Definitional form (limit=100): used passes through.
        let w = window(UsageWindowKind::Rolling, 58, None);
        assert_eq!(shape_window(&w).unwrap()["percent"], json!(58.0));
        // A real cap (future adapter): percent is derived, rounded.
        let mut w2 = window(UsageWindowKind::Weekly, 5_000_000, None);
        w2.limit = Some(10_000_000);
        assert_eq!(shape_window(&w2).unwrap()["percent"], json!(50.0));
        // Zero cap: no entry, never a division by zero or a fake percent.
        let mut w3 = window(UsageWindowKind::Weekly, 1, None);
        w3.limit = Some(0);
        assert!(shape_window(&w3).is_none());
    }
}
