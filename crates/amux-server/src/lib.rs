//! amux server: HTTP API, SQLite store, orchestrator runtime.
//!
//! Module layout mirrors docs/rust-rebuild-plan.md §Crate structure. Modules
//! land phase by phase; each `pub mod` line appears when its RR item starts.

pub mod api;
pub mod backend;
pub mod config;
pub mod db;
pub mod opencode;
pub mod orchestrator;
pub mod provider;
pub mod runtime_jobs;
pub mod tls;

use std::sync::Arc;
use std::time::Instant;

/// Content hash of this binary, computed once at startup. The discriminator
/// that answers "did the server change underneath me" (CLAUDE.md workflow
/// rule; ethos rule 4). Falls back to the compile-time version when the
/// binary path is unreadable.
pub fn build_hash() -> String {
    (|| -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let bytes = std::fs::read(exe).ok()?;
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(&bytes);
        Some(hex::encode(&h.finalize()[..8]))
    })()
    .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")))
}

pub fn run() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async_main());
}

async fn async_main() {
    // rustls refuses to guess when both ring and aws-lc-rs are in the
    // dependency graph (reqwest pulls one, axum-server the other). Pin ring
    // explicitly or the first TLS handshake panics the accept loop.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = config::ServerConfig::from_process_env();
    tracing::info!(port = cfg.port, db = %cfg.db_path.display(), "starting amux-rust");

    let store = match db::Store::open(&cfg.db_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "store open failed");
            std::process::exit(1);
        }
    };

    let auth_token = api::auth::load_or_create_token(&cfg.auth_token_path()).ok();

    let state = api::AppState {
        store: store.clone(),
        started: Instant::now(),
        build_hash: build_hash(),
        auth_token,
    };
    let app = api::router(state);

    let tls = tls::load_or_generate(&cfg.tls_dir()).expect("tls material");
    let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_pem(
        tls.cert_pem.into_bytes(),
        tls.key_pem.into_bytes(),
    )
    .await
    .expect("rustls config");

    // Orchestrator runtime: reconcile once, then tick (RR-0041).
    let runtime = Arc::new(orchestrator::runtime::Runtime {
        store: store.clone(),
        backends: vec![
            Arc::new(backend::tmux::TmuxBackend::new()),
            // Herdr backend joins the reconcile set when a herdr session is
            // configured (AMUX_HERDR_SESSION); constructing it against a
            // missing server would make every probe a failure report.
        ],
        tick_secs: cfg
            .env
            .get("AMUX_RS_TICK_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3),
        heartbeat_every: 10,
        breaker: amux_core::circuit::FleetCircuitBreaker {
            // Spend trip disabled until the token ledger wires in (Phase 4)
            // — 0 budget with 0 accounting would trip instantly on lies.
            window_budget_tokens: u64::MAX,
            window_secs: 3600,
            min_progress_per_window: 0, // no-progress trip opt-in via config later
            max_failures_per_window: 50,
        },
        fleet_state: std::sync::Mutex::new(amux_core::circuit::FleetState::Normal),
        protocol: Some(Arc::new(
            opencode::structured::StructuredCliProtocol::new(),
        )),
    });
    match runtime.reconcile_on_startup().await {
        Ok(report) => tracing::info!(
            interrupted = report.interrupted.len(),
            stale = report.stale_backend.len(),
            probe_failures = report.backend_probe_failures.len(),
            "startup reconciliation complete"
        ),
        Err(e) => tracing::warn!(error = %e, "startup reconciliation failed"),
    }
    tokio::spawn(runtime.run());

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!(%addr, "listening (https)");
    axum_server::bind_rustls(addr, rustls_cfg)
        .serve(app.into_make_service())
        .await
        .expect("server run");
}
