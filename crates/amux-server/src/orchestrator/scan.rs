//! Terminal scan loop with demotion (RR-0067, D1).
//!
//! The scraper is the FALLBACK voice, never the control plane: a worker
//! whose structured protocol session is live is not scanned at all — its
//! own reports outrank any scrape (the D1 exit, applied from day one
//! instead of retrofitted like the Python's AMUX_SCAN_DEMOTE_S). Only
//! workers with a live TERMINAL session and no protocol session get their
//! panes captured and run through the TerminalAdapter.
//!
//! Consecutive identical scan results are deduped by content hash — the
//! Python system's two-scan persistence gate, done once here so every
//! event consumer inherits it (a rate-limit banner sitting in scrollback
//! must not re-fire state changes every cycle).

use crate::backend::{ProcessRef, SessionBackend};
use crate::db::SharedStore;
use crate::opencode::AgentProtocol;
use amux_core::ids::WorkerId;
use amux_core::provider::ProviderId;
use sha2::Digest;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ScanLoop {
    pub store: SharedStore,
    pub backends: Vec<Arc<dyn SessionBackend>>,
    pub protocol: Option<Arc<dyn AgentProtocol>>,
    /// worker -> hash of the last scan's emitted events (dedupe).
    last_scan: Mutex<BTreeMap<WorkerId, String>>,
}

/// What one scan pass did — returned so callers (and tests) see the
/// demotion decisions instead of inferring them from silence (the
/// /api/debug/scan lesson: a skip that leaves no trace is indistinguishable
/// from a scan that found nothing).
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ScanReport {
    pub scanned: Vec<String>,
    pub demoted_structured: Vec<String>,
    pub events_applied: usize,
    pub capture_failures: Vec<String>,
}

impl ScanLoop {
    pub fn new(
        store: SharedStore,
        backends: Vec<Arc<dyn SessionBackend>>,
        protocol: Option<Arc<dyn AgentProtocol>>,
    ) -> Self {
        ScanLoop {
            store,
            backends,
            protocol,
            last_scan: Mutex::new(BTreeMap::new()),
        }
    }

    /// One pass over live terminal sessions.
    pub async fn scan_once(&self) -> anyhow::Result<ScanReport> {
        let mut report = ScanReport::default();
        // Live terminal-backed sessions: (worker_id, backend name, ref, provider).
        let targets: Vec<(String, String, String, String)> = {
            let conn = self.store.read()?;
            let mut stmt = conn.prepare(
                "SELECT s.worker_id, s.backend, s.backend_ref,
                        COALESCE(w.provider, 'claude')
                 FROM _amux_sessions s
                 LEFT JOIN _amux_workers w ON w.id = s.worker_id
                 WHERE s.ended_at IS NULL AND s.backend IN ('tmux', 'herdr')",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };

        for (wid_str, backend_name, backend_ref, provider) in targets {
            let Ok(worker) = WorkerId::parse(&wid_str) else { continue };

            // DEMOTION: a live structured session means the worker speaks
            // for itself. Skip — and SAY so.
            if let Some(p) = &self.protocol {
                if p.state(&worker).await.is_ok() {
                    report.demoted_structured.push(wid_str.clone());
                    continue;
                }
            }

            let Some(backend) = self.backends.iter().find(|b| b.name() == backend_name)
            else {
                continue;
            };
            let proc = ProcessRef {
                backend_ref: backend_ref.clone(),
                pid: None,
            };
            let captured = match backend.capture(&proc, 60).await {
                Ok(c) => c,
                Err(e) => {
                    report.capture_failures.push(format!("{wid_str}: {e}"));
                    continue;
                }
            };
            report.scanned.push(wid_str.clone());

            let adapter =
                crate::backend::adapter::TerminalAdapter::new(ProviderId(provider.clone()));
            let events = adapter.scan(&captured);
            if events.is_empty() {
                continue;
            }
            // Dedupe: same events from the same worker as last pass = the
            // banner is still on screen, not a new occurrence.
            let hash = {
                let mut h = sha2::Sha256::new();
                h.update(serde_json::to_string(&events).unwrap_or_default());
                hex::encode(h.finalize())
            };
            {
                let mut last = self.last_scan.lock().unwrap();
                if last.get(&worker).map(|s| s.as_str()) == Some(hash.as_str()) {
                    continue;
                }
                last.insert(worker.clone(), hash);
            }
            for ev in events {
                let w = worker.clone();
                let applied = self
                    .store
                    .write_async(move |conn| {
                        crate::orchestrator::events::apply_event(
                            conn,
                            &w,
                            &ev,
                            chrono::Utc::now(),
                        )
                    })
                    .await;
                match applied {
                    Ok(_) => report.events_applied += 1,
                    Err(e) => {
                        tracing::warn!(worker = %worker, error = %e, "scan event apply failed")
                    }
                }
            }
        }
        Ok(report)
    }

    /// The loop: scan every `interval_secs`, forever.
    pub async fn run(self: Arc<Self>, interval_secs: u64) {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(5)));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            crate::runtime_jobs::registry::tick(crate::runtime_jobs::registry::ids::SCAN);
            match self.scan_once().await {
                Ok(r) if !r.scanned.is_empty() || !r.capture_failures.is_empty() => {
                    tracing::debug!(
                        scanned = r.scanned.len(),
                        demoted = r.demoted_structured.len(),
                        events = r.events_applied,
                        failures = r.capture_failures.len(),
                        "terminal scan pass"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "terminal scan pass failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AttachInfo, BackendError, BackendSession, BackendStatus, SessionSpec};
    use crate::db::WriteOutcome;
    use crate::opencode::mock::MockProtocol;
    use crate::opencode::AgentState;
    use async_trait::async_trait;
    use rusqlite::params;

    /// Backend whose capture returns a scripted frame.
    struct ScriptedBackend {
        name: &'static str,
        frame: String,
    }

    #[async_trait]
    impl SessionBackend for ScriptedBackend {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn spawn(&self, _s: &SessionSpec) -> crate::backend::Result<ProcessRef> {
            Err(BackendError::SpawnFailed("scripted".into()))
        }
        async fn terminate(&self, _p: &ProcessRef) -> crate::backend::Result<()> {
            Ok(())
        }
        async fn status(&self, _p: &ProcessRef) -> crate::backend::Result<BackendStatus> {
            Ok(BackendStatus::Running)
        }
        async fn attach_info(&self, _p: &ProcessRef) -> crate::backend::Result<AttachInfo> {
            Ok(AttachInfo { command: "true".into() })
        }
        async fn reconcile(&self) -> crate::backend::Result<Vec<BackendSession>> {
            Ok(vec![])
        }
        async fn capture(&self, _p: &ProcessRef, _l: u32) -> crate::backend::Result<String> {
            Ok(self.frame.clone())
        }
    }

    fn store() -> SharedStore {
        let dir = tempfile::tempdir().unwrap();
        let s = Arc::new(crate::db::Store::open(&dir.path().join("t.db")).unwrap());
        std::mem::forget(dir);
        s
    }

    fn wid(n: u128) -> WorkerId {
        WorkerId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, 800 + n))
    }

    fn seed_terminal_worker(store: &SharedStore, w: &WorkerId) {
        let (id, sid) = (w.to_string(), format!("ses_{w}"));
        store
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO _amux_workers (id, display_name, provider, created_at, updated_at)
                     VALUES (?1, 'term', 'claude-code', 'now', 'now')",
                    params![id],
                )?;
                conn.execute(
                    "INSERT INTO _amux_sessions (id, worker_id, backend, backend_ref, started_at)
                     VALUES (?1, ?2, 'tmux', 'amux-x', 'now')",
                    params![sid, id],
                )?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
    }

    // A frame the Claude adapter flags: the weekly limit banner (fixture
    // shape from adapter.rs's corpus).
    const LIMIT_FRAME: &str = "\n\u{23fa} did things\nYou've reached your weekly limit \u{00b7} resets 3pm\n\u{276f} \n";

    #[tokio::test]
    async fn structured_session_is_demoted_not_scanned() {
        let store = store();
        let w = wid(1);
        seed_terminal_worker(&store, &w);
        let protocol = Arc::new(MockProtocol::new());
        protocol.register(w.clone(), AgentState::Idle); // structured = live
        let scan = ScanLoop::new(
            store,
            vec![Arc::new(ScriptedBackend { name: "tmux", frame: LIMIT_FRAME.into() })],
            Some(protocol),
        );
        let r = scan.scan_once().await.unwrap();
        assert_eq!(r.demoted_structured, vec![w.to_string()]);
        assert!(r.scanned.is_empty(), "demoted worker must not be captured");
    }

    #[tokio::test]
    async fn hookless_worker_is_scanned_and_events_dedupe() {
        let store = store();
        let w = wid(2);
        seed_terminal_worker(&store, &w);
        // No protocol session: the scraper is this worker's only voice.
        let scan = ScanLoop::new(
            store.clone(),
            vec![Arc::new(ScriptedBackend { name: "tmux", frame: LIMIT_FRAME.into() })],
            Some(Arc::new(MockProtocol::new())),
        );
        let r1 = scan.scan_once().await.unwrap();
        assert_eq!(r1.scanned, vec![w.to_string()]);
        // Whether the fixture fires depends on the adapter's exact anchors;
        // what MUST hold: a second identical pass applies nothing new.
        let r2 = scan.scan_once().await.unwrap();
        assert_eq!(r2.events_applied, 0, "identical frame must dedupe: {r2:?}");
    }

    #[tokio::test]
    async fn capture_failure_is_reported_not_silent() {
        struct FailingBackend;
        #[async_trait]
        impl SessionBackend for FailingBackend {
            fn name(&self) -> &'static str { "tmux" }
            async fn spawn(&self, _s: &SessionSpec) -> crate::backend::Result<ProcessRef> {
                Err(BackendError::SpawnFailed("x".into()))
            }
            async fn terminate(&self, _p: &ProcessRef) -> crate::backend::Result<()> { Ok(()) }
            async fn status(&self, _p: &ProcessRef) -> crate::backend::Result<BackendStatus> {
                Ok(BackendStatus::NotFound)
            }
            async fn attach_info(&self, _p: &ProcessRef) -> crate::backend::Result<AttachInfo> {
                Ok(AttachInfo { command: "true".into() })
            }
            async fn reconcile(&self) -> crate::backend::Result<Vec<BackendSession>> { Ok(vec![]) }
            async fn capture(&self, _p: &ProcessRef, _l: u32) -> crate::backend::Result<String> {
                Err(BackendError::CommandFailed("pane gone".into()))
            }
        }
        let store = store();
        let w = wid(3);
        seed_terminal_worker(&store, &w);
        let scan = ScanLoop::new(store, vec![Arc::new(FailingBackend)], None);
        let r = scan.scan_once().await.unwrap();
        assert_eq!(r.capture_failures.len(), 1);
        assert!(r.capture_failures[0].contains("pane gone"));
    }
}
