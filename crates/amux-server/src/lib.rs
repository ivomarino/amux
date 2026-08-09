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
        store,
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

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!(%addr, "listening (https)");
    axum_server::bind_rustls(addr, rustls_cfg)
        .serve(app.into_make_service())
        .await
        .expect("server run");
}
