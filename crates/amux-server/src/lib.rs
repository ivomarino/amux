//! amux server: HTTP API, SQLite store, orchestrator runtime.
//!
//! Module layout mirrors docs/rust-rebuild-plan.md §Crate structure. Modules
//! land phase by phase; each `pub mod` line appears when its RR item starts.

pub mod api;
pub mod backend;
pub mod config;
pub mod db;
pub mod integrations;
pub mod opencode;
pub mod orchestrator;
pub mod provider;
pub mod push;
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

    // Migration-rehearsal mode (Phase 11): open + migrate + report + exit.
    // Lets scripts/migration-rehearsal.sh exercise the EXACT production
    // migration path against a DB copy without binding ports.
    if cfg.env.get("AMUX_RS_MIGRATE_ONLY").map(|v| v == "1").unwrap_or(false) {
        let conn = store.read().expect("read after migrate");
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let migrations: i64 = conn
            .query_row("SELECT COUNT(*) FROM _amux_migrations", [], |r| r.get(0))
            .unwrap_or(-1);
        println!(
            "{}",
            serde_json::json!({
                "migrate_only": true,
                "tables": tables,
                "migrations_applied": migrations,
            })
        );
        return;
    }

    let auth_token = api::auth::load_or_create_token(&cfg.auth_token_path()).ok();

    // RR-0092: at boot no amux-launched Chrome exists, so Singleton* locks in
    // amux-owned profile dirs are stale by definition and would block the next
    // launch (AMUX-2070). Only touches ~/.amux/playwright-auth — never the
    // user's real Chrome dir. Logs WHAT was cleaned, not just that it ran.
    for (dir, removed) in integrations::browser::reconcile_locks_at_startup(&cfg.amux_home) {
        tracing::info!(dir = %dir.display(), locks = ?removed, "cleaned stale Chrome profile locks");
    }

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
        pickup_unowned: cfg.env.get("AMUX_RS_PICKUP_UNOWNED").map(|v| v == "1").unwrap_or(false),
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
    tokio::spawn(runtime.clone().run());

    // Terminal scan loop (RR-0067): the fallback voice for hookless
    // interactive workers, with structured-session demotion built in.
    let scan = Arc::new(orchestrator::scan::ScanLoop::new(
        store.clone(),
        runtime.backends.clone(),
        runtime.protocol.clone(),
    ));
    let scan_secs = cfg
        .env
        .get("AMUX_RS_SCAN_SECS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    tokio::spawn(scan.run(scan_secs));

    // Self-adoption (parity with the Python server's own-mtime watch): when
    // the INSTALLED binary changes underneath us — the builder agent just
    // installed a new build — exit 0 and let launchd's KeepAlive relaunch
    // the new code. The binary is the unit of deploy; a server that keeps
    // running stale code after a deploy is the Python shared-checkout
    // staleness incident wearing a compiled coat.
    tokio::spawn(async {
        let Ok(exe) = std::env::current_exe() else { return };
        let Ok(meta) = std::fs::metadata(&exe) else { return };
        let initial = meta.modified().ok();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let current = std::fs::metadata(&exe).ok().and_then(|m| m.modified().ok());
            if current.is_some() && current != initial {
                tracing::info!("binary changed on disk — exiting for relaunch (self-adoption)");
                std::process::exit(0);
            }
        }
    });

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!(%addr, "listening (https)");
    axum_server::bind_rustls(addr, rustls_cfg)
        .serve(app.into_make_service())
        .await
        .expect("server run");
}
