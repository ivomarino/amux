#![recursion_limit = "256"] // 40-key json! literals (session card parity)
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

/// Store-backed [`opencode::structured::ConversationSink`] (AMUX-2613
/// gap 2): captured conversation refs land in `_amux_conversations`
/// (migration 0011) so backend::bootstrap hydrates them back after a
/// restart. Fire-and-forget by contract — a persistence hiccup must never
/// stall a turn's reader task, so failures are logged, not propagated.
/// `applied: false` on purpose: this is protocol plumbing state with no
/// entity/SSE consumer, so it must not bump the global revision (Invariant
/// 37 gates rev on entity-visible change; the write itself still commits).
struct StoreConversationSink {
    store: db::SharedStore,
}

impl opencode::structured::ConversationSink for StoreConversationSink {
    fn save(&self, worker: &amux_core::ids::WorkerId, family: &str, conversation_ref: &str) {
        let store = self.store.clone();
        let (w, f, c) = (worker.to_string(), family.to_string(), conversation_ref.to_string());
        tokio::spawn(async move {
            let (ww, ff, cc) = (w.clone(), f, c);
            let res = store
                .write_async(move |conn| {
                    conn.execute(
                        "INSERT INTO _amux_conversations
                             (worker_id, provider, conversation_ref, updated_at)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(worker_id) DO UPDATE SET
                             provider = ?2, conversation_ref = ?3, updated_at = ?4",
                        rusqlite::params![ww, ff, cc, chrono::Utc::now().to_rfc3339()],
                    )?;
                    Ok(db::WriteOutcome { applied: false, events: vec![] })
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(worker = %w, error = %e, "conversation ref persist failed");
            }
        });
    }

    fn forget(&self, worker: &amux_core::ids::WorkerId) {
        let store = self.store.clone();
        let w = worker.to_string();
        tokio::spawn(async move {
            let ww = w.clone();
            let res = store
                .write_async(move |conn| {
                    conn.execute(
                        "DELETE FROM _amux_conversations WHERE worker_id = ?1",
                        rusqlite::params![ww],
                    )?;
                    Ok(db::WriteOutcome { applied: false, events: vec![] })
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(worker = %w, error = %e, "conversation ref forget failed");
            }
        });
    }
}

async fn async_main() {
    // rustls refuses to guess when both ring and aws-lc-rs are in the
    // dependency graph (reqwest pulls one, axum-server the other). Pin ring
    // explicitly or the first TLS handshake panics the accept loop.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Config first: the log-file path below needs amux_home.
    let cfg = config::ServerConfig::from_process_env();

    // Tracing tees to stdout AND ~/.amux/logs/server-rs.log (AMUX-2605):
    // the file is what GET /api/logs/raw tails — python parity, where the
    // Logs tab's raw view reads the server's own log. ANSI off so the file
    // (and the SPA's raw view) gets clean text. If the file cannot be
    // opened, stdout-only — logging setup must never stop the server.
    let env_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
    };
    let log_file = {
        let dir = cfg.amux_home.join("logs");
        std::fs::create_dir_all(&dir).ok();
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("server-rs.log"))
            .ok()
    };
    match log_file {
        Some(f) => {
            use tracing_subscriber::fmt::writer::MakeWriterExt;
            tracing_subscriber::fmt()
                .with_env_filter(env_filter())
                .with_ansi(false)
                .with_writer(std::io::stdout.and(Arc::new(f)))
                .init()
        }
        None => tracing_subscriber::fmt().with_env_filter(env_filter()).init(),
    }

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

    // AMUX_AUTH_TOKEN parity (amux-server.py:701): a non-empty env value IS
    // the token, the literal "none" disables auth, otherwise the token file
    // shared with the Python server.
    let auth_token = match cfg.env.get("AMUX_AUTH_TOKEN").map(|s| s.trim()) {
        Some(v) if v.eq_ignore_ascii_case("none") => None,
        Some(v) if !v.is_empty() => Some(v.to_string()),
        _ => api::auth::load_or_create_token(&cfg.auth_token_path()).ok(),
    };

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
    // Steering DELIVERY (AMUX-2617). `steer_enqueue` had three call sites and no
    // consumer: queued messages were stored durably and never handed to the lane,
    // so a busy worker's queue only grew (amux-rust: IDLE, 9 QUEUED, oldest 2h6m).
    // Spawned before the router takes `state` by value.
    tokio::spawn(api::session_verbs::steer_deliver_loop(state.clone()));

    let app = api::router(state);

    // SNI dual-cert: Tailscale LE cert for the tailnet hostname, self-signed
    // for localhost/IPs — Python-server parity (its _sni_cb), so the PWA's
    // service worker registers over https://<host>.ts.net:<port>.
    let server_config = tls::build_server_config(&cfg.tls_dir()).expect("tls material");
    let rustls_cfg =
        axum_server::tls_rustls::RustlsConfig::from_config(std::sync::Arc::new(server_config));

    // Terminal backends (RR-0031/0032): tmux always; herdr joins when
    // AMUX_HERDR_SESSION names the herdr session hosting amux workspaces
    // (backend::backends_from_env — constructing it against a missing herdr
    // server would make every probe a failure report).
    let backends = backend::backends_from_env(&cfg.env);
    // Publish the backend set for API handlers (worker peek, AMUX-2613
    // gap 4): AppState is constructed at 40+ sites, so the accessor is a
    // process-wide slot rather than a new required field on every one.
    backend::set_process_backends(backends.clone());
    // ONE protocol instance shared by the runtime pump, the scan demotion
    // check, and the bootstrap registrar — two instances would disagree
    // about which workers have live sessions (ethos rule 4).
    //
    // Conversation refs the protocol captures persist to
    // `_amux_conversations` (AMUX-2613 gap 2) so bootstrap re-hydrates them
    // after a restart — an in-memory-only ref is fiction (the D1 report-
    // table lesson: this process re-execs on every deploy).
    let protocol = Arc::new(opencode::structured::StructuredCliProtocol::with_conversation_sink(
        Arc::new(StoreConversationSink { store: store.clone() }),
    ));

    // Orchestrator runtime: reconcile once, then tick (RR-0041).
    let runtime = Arc::new(orchestrator::runtime::Runtime {
        store: store.clone(),
        backends: backends.clone(),
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
        protocol: Some(protocol.clone() as Arc<dyn opencode::AgentProtocol>),
        pickup_unowned: cfg.env.get("AMUX_RS_PICKUP_UNOWNED").map(|v| v == "1").unwrap_or(false),
        // RR-0044b: staggered un-park interval after a provider rate-limit
        // reset (thundering-herd prevention).
        resume_stagger_secs: cfg
            .env
            .get("AMUX_RS_RESUME_STAGGER_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(amux_core::provider_fleet::DEFAULT_RESUME_STAGGER_SECS),
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

    // Worker event processors (RR-0065, AMUX-2613 gap 1): the durable
    // subscriber to protocol.events(). Without this spawn the whole event
    // module was test-only plumbing — a headless turn ran, completed, and
    // the worker's DB row said `idle` throughout, with turn outcomes
    // visible only in a tracing line (a store the reader never opens is
    // the same failure as no tag, ethos rule 4). One processor per worker
    // with a live session; TurnStarted -> Active{turn}, TurnCompleted ->
    // Idle + command confirmation + the drift-note path, all journaled
    // with payloads (RR-0111a). The scan loop stays the FALLBACK voice:
    // it already demotes any worker whose protocol session is live.
    tokio::spawn(orchestrator::events::run_event_processors(
        store.clone(),
        runtime.protocol.clone().expect("protocol constructed above"),
    ));

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

    // Session bootstrap (backend::bootstrap): the spawn/registration glue
    // that turns the API's durable Starting/ended records into real backend
    // processes and structured-protocol sessions. Without it a started
    // worker never gets a terminal and the pump sees NoSession forever —
    // see the module docs for the incident.
    let boot = Arc::new(backend::bootstrap::Bootstrap {
        store: store.clone(),
        backends,
        registry: Arc::new(provider::default_registry()),
        registrar: protocol,
    });
    let boot_secs = cfg
        .env
        .get("AMUX_RS_BOOTSTRAP_SECS")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    tokio::spawn(boot.run(boot_secs));

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

    // LEGACY PORT TAKEOVER (python retirement, 2026-08-09): every running
    // fleet session carries AMUX_URL=https://localhost:8822 baked into its
    // process env — env vars in live processes cannot be rotated, so the
    // moment the python server stopped, the whole fleet's board/API calls
    // started failing. The rust server therefore answers the OLD port too:
    // same router, same TLS, same auth. Gated on AMUX_RS_LEGACY_PORT so a
    // deliberate python resurrection (unset it) gets the port back without a
    // code change. Set to 8822 in ~/.amux/server.env.
    if let Some(legacy_port) = std::env::var("AMUX_RS_LEGACY_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
    {
        let legacy_addr = std::net::SocketAddr::from(([0, 0, 0, 0], legacy_port));
        let legacy_acceptor = tls::RedirectingAcceptor::new(
            axum_server::tls_rustls::RustlsAcceptor::new(rustls_cfg.clone()),
            format!("localhost:{legacy_port}"),
        );
        let legacy_app = app.clone();
        tracing::info!(%legacy_addr, "listening on LEGACY port (fleet AMUX_URL compatibility)");
        tokio::spawn(async move {
            if let Err(e) = axum_server::bind(legacy_addr)
                .acceptor(legacy_acceptor)
                .serve(
                    legacy_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .await
            {
                // Port taken (python running?) — log loudly, keep the primary.
                tracing::error!(error = %e, port = legacy_port, "legacy port bind failed");
            }
        });
    }

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!(%addr, "listening (https, plain-http redirected)");
    let acceptor = tls::RedirectingAcceptor::new(
        axum_server::tls_rustls::RustlsAcceptor::new(rustls_cfg),
        format!("localhost:{}", cfg.port),
    );
    axum_server::bind(addr)
        .acceptor(acceptor)
        // with_connect_info: the auth middleware's localhost bypass (Python
        // parity) needs the PEER address; without this every request looks
        // remote and local tokenless CLI calls 401.
        .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .await
        .expect("server run");
}
