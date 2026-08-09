//! Orchestrator runtime (RR-0041, Invariants 9, 10, 11) + FleetProgress
//! heartbeat (Lesson L4).
//!
//! Drives the pure planner (`amux_core::orchestrator::plan_tick`) on an
//! interval, executes its plan against the store, and reconciles DB state
//! against backend reality at startup. Task assembly arrives with the board
//! API (Phase 2, RR-0049) — until then the planner runs over an empty task
//! list, which still exercises lease reclaim and the heartbeat.

use crate::backend::{BackendStatus, SessionBackend};
use crate::db::{PendingEvent, SharedStore, WriteOutcome};
use amux_core::orchestrator::{plan_tick, Lease, TickInputs, TickPlan};
use amux_core::revision::{EntityType, MutationKind};
use amux_core::worker::Worker;
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The L4 heartbeat: "is progress continuing?" answered as data, not
/// inferred from scrollback. Published as a StateEvent so SSE clients and
/// the dashboard status bar receive it like any other state change.
#[derive(Debug, Clone, Serialize)]
pub struct FleetProgress {
    pub at: DateTime<Utc>,
    pub workers_total: usize,
    pub workers_active: usize,
    pub live_leases: usize,
    pub reclaimed_last_tick: usize,
    pub stall_violations: usize,
    pub quarantined_total: u64,
}

pub struct Runtime {
    pub store: SharedStore,
    pub backends: Vec<Arc<dyn SessionBackend>>,
    pub tick_secs: u64,
    /// Heartbeat cadence in ticks (heartbeat every Nth tick).
    pub heartbeat_every: u64,
}

impl Runtime {
    /// Startup reconciliation (Invariant 9): the DB's picture of live
    /// sessions vs what each backend actually hosts. Every mismatch becomes
    /// a StateEvent — reported, never silently patched over.
    pub async fn reconcile_on_startup(&self) -> anyhow::Result<ReconcileReport> {
        let mut report = ReconcileReport::default();

        // What the backends actually host.
        let mut backend_refs: BTreeMap<String, BackendStatus> = BTreeMap::new();
        for b in &self.backends {
            match b.reconcile().await {
                Ok(sessions) => {
                    for s in sessions {
                        backend_refs.insert(s.backend_ref, s.status);
                    }
                }
                Err(e) => {
                    // A backend that cannot answer is reported, not skipped
                    // silently — its sessions would all read as "missing"
                    // and mass-ending them on a flaky probe would be the
                    // reaper incident all over again.
                    report.backend_probe_failures.push(format!("{}: {e}", b.name()));
                }
            }
        }
        let probe_ok = report.backend_probe_failures.is_empty();

        // DB live sessions vs backend truth.
        let db_live: Vec<(String, String, String)> = {
            let conn = self.store.read()?;
            let mut stmt = conn.prepare(
                "SELECT id, worker_id, backend_ref FROM _amux_sessions WHERE ended_at IS NULL",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };

        for (session_id, worker_id, backend_ref) in db_live {
            let live_in_backend = matches!(
                backend_refs.get(&backend_ref),
                Some(BackendStatus::Running)
            );
            if !live_in_backend && probe_ok {
                // DB says running, backend says gone -> mark interrupted.
                report.interrupted.push(worker_id.clone());
                let sid = session_id.clone();
                self.store
                    .write_async(move |conn| {
                        conn.execute(
                            "UPDATE _amux_sessions SET ended_at = ?1, exit_reason = ?2
                             WHERE id = ?3 AND ended_at IS NULL",
                            params![
                                Utc::now().to_rfc3339(),
                                serde_json::json!({"reason": "crashed", "signal": null})
                                    .to_string(),
                                sid
                            ],
                        )?;
                        Ok(WriteOutcome {
                            applied: true,
                            events: vec![PendingEvent {
                                entity_type: EntityType::Session,
                                entity_id: session_id.clone(),
                                mutation: MutationKind::StatusChanged {
                                    from: "running".into(),
                                    to: "interrupted".into(),
                                },
                            }],
                        })
                    })
                    .await?;
            }
        }

        // Backend hosts an amux ref the DB has no live row for -> stale
        // process, reported for a human/next phase to adopt or kill (ethos
        // rule 8: it may be someone's live work — never auto-kill on sight).
        let db_refs: std::collections::BTreeSet<String> = {
            let conn = self.store.read()?;
            let mut stmt = conn
                .prepare("SELECT backend_ref FROM _amux_sessions WHERE ended_at IS NULL")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for (bref, status) in &backend_refs {
            if matches!(status, BackendStatus::Running) && !db_refs.contains(bref) {
                report.stale_backend.push(bref.clone());
            }
        }

        Ok(report)
    }

    /// The tick loop. Runs forever; errors are logged and the loop
    /// continues — a failed tick must not kill the orchestrator.
    pub async fn run(self: Arc<Self>) {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(self.tick_secs.max(1)));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut tick_n: u64 = 0;
        loop {
            interval.tick().await;
            tick_n += 1;
            let heartbeat = tick_n.is_multiple_of(self.heartbeat_every.max(1));
            if let Err(e) = self.tick_once(heartbeat).await {
                tracing::warn!(error = %e, "orchestrator tick failed");
            }
        }
    }

    /// One tick: load state, plan, execute the plan.
    pub async fn tick_once(&self, heartbeat: bool) -> anyhow::Result<()> {
        let now = Utc::now();
        let (workers, leases, quarantined_total) = self.load_state()?;

        // Tasks arrive with the board API (Phase 2). Empty until then.
        let tasks: Vec<amux_core::board::Task> = vec![];
        let hints = BTreeMap::new();
        let attempts = BTreeMap::new();
        let fleet_state = amux_core::circuit::FleetState::Normal;

        let plan = plan_tick(&TickInputs {
            now,
            tasks: &tasks,
            workers: &workers,
            leases: &leases,
            fleet_state: &fleet_state,
            hints: &hints,
            attempts: &attempts,
            gates: &[],
            lease_secs: 600,
            wip_limit: 1,
        });

        self.execute(&plan).await?;

        if heartbeat {
            let progress = FleetProgress {
                at: now,
                workers_total: workers.len(),
                workers_active: workers
                    .iter()
                    .filter(|w| {
                        matches!(w.state, amux_core::worker::WorkerState::Active { .. })
                    })
                    .count(),
                live_leases: leases.iter().filter(|l| !l.is_expired(now)).count(),
                reclaimed_last_tick: plan.reclaim.len(),
                stall_violations: plan.stalls.len(),
                quarantined_total,
            };
            let payload = serde_json::to_string(&progress)?;
            self.store
                .write_async(move |_conn| {
                    Ok(WriteOutcome {
                        applied: true,
                        events: vec![PendingEvent {
                            entity_type: EntityType::Other("fleet_progress".into()),
                            entity_id: payload.clone(),
                            mutation: MutationKind::Updated,
                        }],
                    })
                })
                .await?;
        }
        Ok(())
    }

    fn load_state(&self) -> anyhow::Result<(Vec<Worker>, Vec<Lease>, u64)> {
        let conn = self.store.read()?;
        let (rows, _total) = crate::db::queries::list_workers(&conn, 10_000, 0)?;
        let workers: Vec<Worker> = rows
            .into_iter()
            .filter_map(|row| {
                let id = amux_core::ids::WorkerId::parse(&row.id).ok()?;
                let mut w = Worker::new(id, row.config(), Default::default());
                w.state = row.state.clone();
                w.version = row.version;
                Some(w)
            })
            .collect();
        let mut stmt = conn.prepare(
            "SELECT task_id, worker_id, acquired_at, expires_at, generation FROM _amux_leases",
        )?;
        let leases: Vec<Lease> = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })?
            .filter_map(|row| {
                let (task, worker, acq, exp, generation) = row.ok()?;
                Some(Lease {
                    task: amux_core::ids::TaskId::parse(&task).ok()?,
                    worker: amux_core::ids::WorkerId::parse(&worker).ok()?,
                    acquired_at: acq.parse().ok()?,
                    expires_at: exp.parse().ok()?,
                    generation,
                })
            })
            .collect();
        let quarantined: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM issues WHERE status = 'quarantined'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok((workers, leases, quarantined))
    }

    async fn execute(&self, plan: &TickPlan) -> anyhow::Result<()> {
        // Reclaim expired leases: delete the row, bump generation via the
        // task's next lease. Each reclaim is an event — a lease that
        // vanishes silently is a diagnosis that can't be made (ethos 4).
        for lease in &plan.reclaim {
            let task = lease.task.to_string();
            let worker = lease.worker.to_string();
            let generation = lease.generation;
            self.store
                .write_async(move |conn| {
                    let n = conn.execute(
                        "DELETE FROM _amux_leases WHERE task_id = ?1 AND generation = ?2",
                        params![task, generation],
                    )?;
                    Ok(WriteOutcome {
                        applied: n > 0,
                        events: if n > 0 {
                            vec![PendingEvent {
                                entity_type: EntityType::Other("lease".into()),
                                entity_id: task.clone(),
                                mutation: MutationKind::StatusChanged {
                                    from: format!("held:{worker}"),
                                    to: "reclaimed".into(),
                                },
                            }]
                        } else {
                            vec![]
                        },
                    })
                })
                .await?;
        }
        // Assignments: written as leases. Prompt delivery through
        // AgentProtocol wires in with RR-0030's transport.
        for asg in &plan.assignments {
            let task = asg.task.to_string();
            let worker = asg.worker.to_string();
            let acquired = asg.lease.acquired_at.to_rfc3339();
            let expires = asg.lease.expires_at.to_rfc3339();
            self.store
                .write_async(move |conn| {
                    let n = conn.execute(
                        // INSERT OR IGNORE: the primary key on task_id is the
                        // atomic claim — a concurrent claimant loses cleanly.
                        "INSERT OR IGNORE INTO _amux_leases
                         (task_id, worker_id, acquired_at, expires_at, generation)
                         VALUES (?1, ?2, ?3, ?4,
                                 COALESCE((SELECT generation + 1 FROM _amux_leases WHERE task_id = ?1), 0))",
                        params![task, worker, acquired, expires],
                    )?;
                    Ok(WriteOutcome {
                        applied: n > 0,
                        events: if n > 0 {
                            vec![PendingEvent {
                                entity_type: EntityType::Other("lease".into()),
                                entity_id: task.clone(),
                                mutation: MutationKind::Created,
                            }]
                        } else {
                            vec![]
                        },
                    })
                })
                .await?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ReconcileReport {
    /// Workers whose DB session was live but whose backend process is gone.
    pub interrupted: Vec<String>,
    /// Backend refs running with no live DB session row.
    pub stale_backend: Vec<String>,
    /// Backends that could not be probed (their sessions were NOT judged).
    pub backend_probe_failures: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{
        AttachInfo, BackendError, BackendSession, ProcessRef, SessionSpec,
    };
    use async_trait::async_trait;

    /// Scripted fake backend for reconciliation tests (Invariant 22).
    struct FakeBackend {
        hosted: Vec<BackendSession>,
        fail_probe: bool,
    }

    #[async_trait]
    impl SessionBackend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn spawn(&self, _s: &SessionSpec) -> crate::backend::Result<ProcessRef> {
            Err(BackendError::SpawnFailed("fake".into()))
        }
        async fn terminate(&self, _p: &ProcessRef) -> crate::backend::Result<()> {
            Ok(())
        }
        async fn status(&self, _p: &ProcessRef) -> crate::backend::Result<BackendStatus> {
            Ok(BackendStatus::NotFound)
        }
        async fn attach_info(&self, _p: &ProcessRef) -> crate::backend::Result<AttachInfo> {
            Ok(AttachInfo {
                command: "true".into(),
            })
        }
        async fn reconcile(&self) -> crate::backend::Result<Vec<BackendSession>> {
            if self.fail_probe {
                Err(BackendError::CommandFailed("probe down".into()))
            } else {
                Ok(self.hosted.clone())
            }
        }
        async fn capture(&self, _p: &ProcessRef, _l: u32) -> crate::backend::Result<String> {
            Ok(String::new())
        }
    }

    fn store() -> SharedStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let s = Arc::new(crate::db::Store::open(&path).unwrap());
        // Leak tempdir so the DB survives the test body.
        std::mem::forget(dir);
        s
    }

    fn seed_live_session(store: &SharedStore, sid: &str, wid: &str, bref: &str) {
        let (sid, wid, bref) = (sid.to_string(), wid.to_string(), bref.to_string());
        store
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO _amux_workers (id, display_name, created_at, updated_at)
                     VALUES (?1, 'w', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    params![wid],
                )?;
                conn.execute(
                    "INSERT INTO _amux_sessions (id, worker_id, backend, backend_ref, started_at)
                     VALUES (?1, ?2, 'fake', ?3, '2026-01-01T00:00:00Z')",
                    params![sid, wid, bref],
                )?;
                Ok(WriteOutcome {
                    applied: true,
                    events: vec![],
                })
            })
            .unwrap();
    }

    #[tokio::test]
    async fn reconcile_marks_vanished_sessions_interrupted() {
        let store = store();
        seed_live_session(&store, "ses_a", "wrk_a", "amux-wrk_a");
        let rt = Runtime {
            store: store.clone(),
            backends: vec![Arc::new(FakeBackend {
                hosted: vec![], // backend hosts nothing
                fail_probe: false,
            })],
            tick_secs: 3,
            heartbeat_every: 10,
        };
        let report = rt.reconcile_on_startup().await.unwrap();
        assert_eq!(report.interrupted, vec!["wrk_a".to_string()]);
        // The session row is ended.
        let conn = store.read().unwrap();
        let ended: Option<String> = conn
            .query_row("SELECT ended_at FROM _amux_sessions WHERE id='ses_a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(ended.is_some());
    }

    #[tokio::test]
    async fn reconcile_reports_stale_backend_refs_without_killing() {
        let store = store();
        let rt = Runtime {
            store,
            backends: vec![Arc::new(FakeBackend {
                hosted: vec![BackendSession {
                    backend_ref: "amux-wrk_ghost".into(),
                    status: BackendStatus::Running,
                }],
                fail_probe: false,
            })],
            tick_secs: 3,
            heartbeat_every: 10,
        };
        let report = rt.reconcile_on_startup().await.unwrap();
        assert_eq!(report.stale_backend, vec!["amux-wrk_ghost".to_string()]);
    }

    #[tokio::test]
    async fn failed_probe_never_mass_ends_sessions() {
        let store = store();
        seed_live_session(&store, "ses_b", "wrk_b", "amux-wrk_b");
        let rt = Runtime {
            store: store.clone(),
            backends: vec![Arc::new(FakeBackend {
                hosted: vec![],
                fail_probe: true, // probe down != sessions gone
            })],
            tick_secs: 3,
            heartbeat_every: 10,
        };
        let report = rt.reconcile_on_startup().await.unwrap();
        assert!(report.interrupted.is_empty(), "flaky probe must not reap");
        assert_eq!(report.backend_probe_failures.len(), 1);
        let conn = store.read().unwrap();
        let ended: Option<String> = conn
            .query_row("SELECT ended_at FROM _amux_sessions WHERE id='ses_b'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(ended.is_none(), "session must survive a failed probe");
    }

    #[tokio::test]
    async fn tick_reclaims_expired_lease_and_heartbeats() {
        let store = store();
        store
            .write(|conn| {
                conn.execute(
                    "INSERT INTO _amux_leases (task_id, worker_id, acquired_at, expires_at, generation)
                     VALUES ('tsk_01JGXV0000000000000000TEST', 'wrk_01JGXV0000000000000000TEST', '2026-01-01T00:00:00Z', '2026-01-01T00:10:00Z', 2)",
                    [],
                )?;
                Ok(WriteOutcome { applied: true, events: vec![] })
            })
            .unwrap();
        let rt = Runtime {
            store: store.clone(),
            backends: vec![],
            tick_secs: 3,
            heartbeat_every: 1,
        };
        let mut rx = store.subscribe();
        rt.tick_once(true).await.unwrap();
        // Lease is gone (expired long ago).
        let conn = store.read().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM _amux_leases", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        drop(conn);
        // Both the reclaim event and the heartbeat were published.
        let mut kinds = vec![];
        while let Ok(ev) = rx.try_recv() {
            kinds.push(format!("{:?}", ev.entity_type));
        }
        assert!(kinds.iter().any(|k| k.contains("lease")), "{kinds:?}");
        assert!(kinds.iter().any(|k| k.contains("fleet_progress")), "{kinds:?}");
    }
}
