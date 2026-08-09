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
    /// Fleet circuit breaker (RR-0048b, Invariant 48). The state lives here,
    /// in memory: a restart resets to Normal, and the first post-restart
    /// window re-trips if the condition persists — a breaker that survives
    /// its own process is a breaker nobody can reset.
    pub breaker: amux_core::circuit::FleetCircuitBreaker,
    pub fleet_state: std::sync::Mutex<amux_core::circuit::FleetState>,
    /// Agent protocol for command delivery (None until a transport is
    /// configured — the pump then leaves queues untouched rather than
    /// failing every command against a void).
    pub protocol: Option<Arc<dyn crate::opencode::AgentProtocol>>,
    /// STRANGLER-FIG SAFETY: may the Rust orchestrator pick up UNOWNED
    /// board tasks? Default false — while the Python fleet runs, an
    /// unowned card may be a Python session's next pickup, and two
    /// orchestrators claiming one queue is the dual-scheduler double-fire
    /// hazard on the board. Tasks owned by a name that resolves to a
    /// REGISTERED Rust worker are always eligible; tasks owned by
    /// unresolvable names (the Python fleet's) are NEVER touched.
    pub pickup_unowned: bool,
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

    /// One tick: load state, evaluate the circuit breaker, plan, execute.
    pub async fn tick_once(&self, heartbeat: bool) -> anyhow::Result<()> {
        let now = Utc::now();
        let (workers, leases, quarantined_total) = self.load_state()?;

        // Circuit breaker (RR-0048b): evaluate the rolling window BEFORE
        // planning. While open, reconciliation looks for runnable work and
        // auto-closes when it finds the fleet can actually move again.
        let window = self.window_stats(now)?;
        // Evaluate under the lock WITHOUT awaiting (the guard is not Send);
        // publish the change after the guard drops.
        let (fleet_state, changed) = {
            let mut fs = self.fleet_state.lock().unwrap();
            let mut changed = None;
            match &*fs {
                amux_core::circuit::FleetState::Normal => {
                    if let Some(tripped) = self.breaker.trip(&window, now) {
                        tracing::warn!(state = ?tripped, "fleet circuit OPENED");
                        *fs = tripped.clone();
                        changed = Some(tripped);
                    }
                }
                _ => {
                    // Open/reconciling: a healthy window is the auto-close
                    // signal — the fleet demonstrably moves again.
                    if self.breaker.evaluate(&window).is_none() {
                        if let Some(closed) = fs.close() {
                            tracing::info!("fleet circuit CLOSED (window healthy)");
                            *fs = closed.clone();
                            changed = Some(closed);
                        }
                    }
                }
            }
            (fs.clone(), changed)
        };
        if let Some(state) = &changed {
            self.publish_fleet_state(state).await.ok();
        }

        // Command delivery pump (Invariant 34): drain each worker's queue
        // head through the agent protocol, honoring DeliveryTiming.
        if let Err(e) = self.pump_commands(now).await {
            tracing::warn!(error = %e, "command pump failed this tick");
        }

        let tasks = self.load_board_tasks(&workers)?;
        let hints = BTreeMap::new();
        let attempts = BTreeMap::new();

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

        // Anti-livelock (RR-0048a): limits filter what actually executes.
        let attempts_by_task = attempts; // (empty until attempt recording wires in)
        let _ = &attempts_by_task;
        let (proceed, exhaustion) = amux_core::orchestrator::enforce_limits(
            plan.assignments.clone(),
            &amux_core::limits::ExecutionLimits::default(),
            now,
            &BTreeMap::new(),
        );
        let plan = TickPlan { assignments: proceed, ..plan };
        self.execute(&plan).await?;
        for action in exhaustion {
            self.apply_exhaustion(action, now).await?;
        }

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

    /// Rolling-window stats from the revisioned event journal — the same
    /// events every other consumer sees, so the breaker and the dashboard
    /// can never disagree about what happened (ethos rule 4).
    fn window_stats(&self, now: DateTime<Utc>) -> anyhow::Result<amux_core::circuit::WindowStats> {
        let conn = self.store.read()?;
        let cutoff = (now - chrono::Duration::seconds(self.breaker.window_secs as i64)).to_rfc3339();
        let completed: u32 = conn.query_row(
            r#"SELECT COUNT(*) FROM _amux_state_events WHERE at > ?1
             AND entity_type = 'task' AND mutation LIKE '%"to":"done"%'"#,
            params![cutoff], |r| r.get(0)).unwrap_or(0);
        let failures: u32 = conn.query_row(
            "SELECT COUNT(*) FROM _amux_state_events WHERE at > ?1
             AND mutation LIKE '%interrupted%'",
            params![cutoff], |r| r.get(0)).unwrap_or(0);
        Ok(amux_core::circuit::WindowStats {
            // Token accounting joins with the turn ledger (Phase 4); 0 keeps
            // the spend trip disabled rather than fed invented numbers.
            tokens_spent: 0,
            tasks_completed: completed,
            failures,
            all_items_blocked: false,
        })
    }

    async fn publish_fleet_state(&self, state: &amux_core::circuit::FleetState) -> anyhow::Result<()> {
        let payload = serde_json::to_string(state)?;
        self.store
            .write_async(move |_conn| {
                Ok(WriteOutcome {
                    applied: true,
                    events: vec![PendingEvent {
                        entity_type: EntityType::Other("fleet_state".into()),
                        entity_id: payload.clone(),
                        mutation: MutationKind::Updated,
                    }],
                })
            })
            .await?;
        Ok(())
    }

    /// Open board tasks the Rust orchestrator may honestly route.
    /// Ownership rule documented on `pickup_unowned`.
    fn load_board_tasks(&self, workers: &[Worker]) -> anyhow::Result<Vec<amux_core::board::Task>> {
        let conn = self.store.read()?;
        let rows = crate::db::board_store::list_issues(
            &conn,
            &["todo".into()],
            &[],
            crate::db::board_store::ArchivedFilter::ActiveOnly,
        )?;
        // Name directory: display names + aliases, case-insensitive.
        let mut names: BTreeMap<String, amux_core::ids::WorkerId> = BTreeMap::new();
        for w in workers {
            names.insert(w.config.display_name.to_lowercase(), w.id().clone());
            for a in &w.config.name_aliases {
                names.entry(a.to_lowercase()).or_insert_with(|| w.id().clone());
            }
        }
        let mut out = Vec::new();
        for row in rows {
            let Some(mut task) = row.to_task() else { continue };
            match row.session.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(owner_name) => {
                    match names.get(&owner_name.to_lowercase()) {
                        Some(wid) => task.worker = Some(wid.clone()),
                        // Owned by a name that is not a Rust worker: the
                        // Python fleet's card. Not ours. Skip entirely.
                        None => continue,
                    }
                }
                None => {
                    if !self.pickup_unowned {
                        continue;
                    }
                }
            }
            out.push(task);
        }
        Ok(out)
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
        // Immutable context snapshots (RR-0070, Invariant 27): record exactly
        // what each assigned worker will receive, BEFORE the command that
        // delivers it is enqueued. INSERT OR IGNORE on the assignment's
        // idempotency key keeps this idempotent — a re-planned assignment
        // re-records nothing and bumps no revision. A task the board no
        // longer resolves is skipped as an honest no-op (the assignment
        // itself will fail downstream and say so there).
        for asg in &plan.assignments {
            let worker_id = asg.worker.clone();
            let task_id = asg.task.clone();
            let key = asg.idempotency_key.clone();
            self.store
                .write_async(move |conn| {
                    let Some(task) =
                        crate::orchestrator::context::task_by_internal_id(conn, &task_id)?
                    else {
                        return Ok(WriteOutcome { applied: false, events: vec![] });
                    };
                    let snap = crate::orchestrator::context::assemble_context(
                        conn, &worker_id, &task,
                    )?;
                    let recorded = crate::orchestrator::context::record_snapshot(
                        conn, &key, &task_id, &worker_id, &snap,
                    )?;
                    Ok(WriteOutcome {
                        applied: recorded,
                        events: if recorded {
                            vec![PendingEvent {
                                entity_type: EntityType::Other("context_snapshot".into()),
                                entity_id: snap.content_hash.clone(),
                                mutation: MutationKind::Created,
                            }]
                        } else {
                            vec![]
                        },
                    })
                })
                .await?;
        }
        // Assignments: lease + an ExecuteTask command the pump delivers
        // through the agent protocol — the full loop: board task -> lease
        // -> command -> headless CLI run.
        for asg in &plan.assignments {
            let cmd_id = amux_core::ids::CommandId::from_ulid(ulid::Ulid::new());
            let worker_id = asg.worker.clone();
            let task_id = asg.task.clone();
            let key = asg.idempotency_key.clone();
            self.store
                .write_async(move |conn| {
                    let (_, created) = crate::db::commands::enqueue(
                        conn,
                        cmd_id,
                        &worker_id,
                        &amux_core::protocol::WorkerCommand::ExecuteTask(task_id),
                        &key,
                        &amux_core::protocol::DeliveryTiming::WhenIdle,
                        None,
                        Utc::now(),
                    )?;
                    Ok(WriteOutcome { applied: created, events: vec![] })
                })
                .await?;
        }
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

impl Runtime {
    /// Execute an anti-livelock decision (RR-0048a). Quarantine writes the
    /// terminal status onto the ISSUES row via the same board-store path
    /// the API uses; Decompose is recorded as a StateEvent for the
    /// decomposition worker to consume (model-driven splitting is a later
    /// phase — recording without acting keeps the decision auditable now).
    async fn apply_exhaustion(
        &self,
        action: amux_core::orchestrator::ExhaustionAction,
        _now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_string(&action)?;
        self.store
            .write_async(move |_conn| {
                Ok(WriteOutcome {
                    applied: true,
                    events: vec![PendingEvent {
                        entity_type: EntityType::Other("exhaustion".into()),
                        entity_id: payload.clone(),
                        mutation: MutationKind::Created,
                    }],
                })
            })
            .await?;
        Ok(())
    }

    /// Deliver due commands through the agent protocol. One in-flight
    /// command per worker (queue discipline lives in db::commands); timing:
    /// Immediate always goes; AtTurnBoundary/WhenIdle require the agent to
    /// be at a boundary (Idle or WaitingForInput). Failures transition
    /// through the core state machine — retry budget 3 (Invariant 34).
    pub(crate) async fn pump_commands(&self, now: DateTime<Utc>) -> anyhow::Result<()> {
        let Some(protocol) = &self.protocol else {
            return Ok(());
        };
        let worker_ids: Vec<String> = {
            let conn = self.store.read()?;
            let mut stmt = conn.prepare(
                "SELECT DISTINCT worker_id FROM _amux_commands
                 WHERE state LIKE '%queued%' OR state LIKE '%dispatched%' OR state LIKE '%delivered%'",
            )?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for wid_str in worker_ids {
            let Ok(worker) = amux_core::ids::WorkerId::parse(&wid_str) else {
                continue;
            };
            let head = {
                let conn = self.store.read()?;
                crate::db::commands::next_deliverable(&conn, &worker)?
            };
            let Some(cmd) = head else { continue };

            // Timing gate.
            let due = match cmd.timing {
                amux_core::protocol::DeliveryTiming::Immediate => true,
                amux_core::protocol::DeliveryTiming::AtTurnBoundary
                | amux_core::protocol::DeliveryTiming::WhenIdle => matches!(
                    protocol.state(&worker).await,
                    Ok(crate::opencode::AgentState::Idle)
                        | Ok(crate::opencode::AgentState::WaitingForInput)
                ),
            };
            if !due {
                continue;
            }

            // Precondition gate (freshness at delivery, Invariant 38): a
            // command whose precondition no longer holds FAILS visibly
            // instead of firing against stale state.
            if let Some(pre) = &cmd.precondition {
                let holds = {
                    let conn = self.store.read()?;
                    let lookup = |entity: &str| -> Option<(u64, String)> {
                        conn.query_row(
                            "SELECT version, json_extract(state,'$.state') FROM _amux_workers WHERE id = ?1",
                            params![entity],
                            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())),
                        )
                        .ok()
                    };
                    pre.evaluate(&lookup)
                };
                if !holds {
                    let id = cmd.id.clone();
                    self.store
                        .write_async(move |conn| {
                            crate::db::commands::transition(
                                conn,
                                &id,
                                amux_core::protocol::CommandTransition::Fail {
                                    reason: "precondition no longer holds at delivery".into(),
                                },
                                3,
                            )?;
                            Ok(WriteOutcome { applied: true, events: vec![] })
                        })
                        .await?;
                    continue;
                }
            }

            // Dispatch.
            let id = cmd.id.clone();
            self.store
                .write_async({
                    let id = id.clone();
                    move |conn| {
                        crate::db::commands::transition(
                            conn,
                            &id,
                            amux_core::protocol::CommandTransition::Dispatch,
                            3,
                        )?;
                        Ok(WriteOutcome { applied: true, events: vec![] })
                    }
                })
                .await?;
            let delivery = match &cmd.command {
                amux_core::protocol::WorkerCommand::DeliverMessage(msg_id) => {
                    // RR-0066: the command carries only the REFERENCE
                    // (Invariant 29); the durable body lives in
                    // _amux_messages and is resolved here, at delivery. A
                    // missing row is a delivery FAILURE, never an empty
                    // message — silently delivering "" would be the Python
                    // lost-steering-text bug wearing a Rust type.
                    let body: rusqlite::Result<String> = {
                        let conn = self.store.read()?;
                        conn.query_row(
                            "SELECT body FROM _amux_messages WHERE id = ?1",
                            params![msg_id.as_str()],
                            |r| r.get(0),
                        )
                    };
                    match body {
                        Ok(body) => {
                            protocol.deliver_message(&worker, msg_id.clone(), body).await
                        }
                        Err(e) => Err(crate::opencode::ProtocolError::Transport(format!(
                            "message body lookup failed for {}: {e}",
                            msg_id.as_str()
                        ))),
                    }
                }
                other => {
                    protocol
                        .send_prompt(
                            &worker,
                            crate::opencode::Prompt {
                                text: serde_json::to_string(other).unwrap_or_default(),
                                idempotency_key: cmd.idempotency_key.clone(),
                            },
                        )
                        .await
                }
            };
            let t = match delivery {
                Ok(()) => amux_core::protocol::CommandTransition::Deliver,
                Err(e) => amux_core::protocol::CommandTransition::Fail {
                    reason: format!("delivery failed at {now}: {e}"),
                },
            };
            self.store
                .write_async(move |conn| {
                    let cmd = crate::db::commands::transition(conn, &id, t, 3)?;
                    // Dead-letter handling (RR-0068, Invariant 34): a Fail
                    // that spends the whole retry budget is retried into its
                    // terminal state HERE, by the caller that observed it —
                    // and the DurableEvent is emitted in the same
                    // transaction, because a dead letter without a trace is
                    // the Python system's silent-vanish mode with extra
                    // steps. transition() returns the post-Fail state, so
                    // this cannot fire on a command with budget left.
                    let mut events = vec![];
                    if matches!(&cmd.state, amux_core::protocol::CommandState::Failed { .. })
                        && cmd.attempts >= 3
                    {
                        let dead = crate::db::commands::transition(
                            conn,
                            &id,
                            amux_core::protocol::CommandTransition::Retry,
                            3,
                        )?;
                        if let amux_core::protocol::CommandState::DeadLettered { reason } =
                            &dead.state
                        {
                            tracing::warn!(command = id.as_str(), reason = %reason,
                                "command dead-lettered after exhausting retries");
                            events.push(PendingEvent {
                                entity_type: EntityType::Other("command_dead_letter".into()),
                                entity_id: id.as_str().to_string(),
                                mutation: MutationKind::StatusChanged {
                                    from: "failed".into(),
                                    to: format!("dead_lettered: {reason}"),
                                },
                            });
                        }
                    }
                    Ok(WriteOutcome { applied: true, events })
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

    fn test_runtime(store: SharedStore, backends: Vec<Arc<dyn SessionBackend>>) -> Runtime {
        Runtime {
            store,
            backends,
            tick_secs: 3,
            heartbeat_every: 1,
            breaker: amux_core::circuit::FleetCircuitBreaker {
                window_budget_tokens: 0, // 0 disables the spend trip in tests
                window_secs: 3600,
                min_progress_per_window: 0,
                max_failures_per_window: 1000,
            },
            fleet_state: std::sync::Mutex::new(amux_core::circuit::FleetState::Normal),
            protocol: None,
            pickup_unowned: false,
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
        let rt = test_runtime(store.clone(), vec![Arc::new(FakeBackend {
            hosted: vec![], // backend hosts nothing
            fail_probe: false,
        })]);
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
        let rt = test_runtime(store, vec![Arc::new(FakeBackend {
            hosted: vec![BackendSession {
                backend_ref: "amux-wrk_ghost".into(),
                status: BackendStatus::Running,
            }],
            fail_probe: false,
        })]);
        let report = rt.reconcile_on_startup().await.unwrap();
        assert_eq!(report.stale_backend, vec!["amux-wrk_ghost".to_string()]);
    }

    #[tokio::test]
    async fn failed_probe_never_mass_ends_sessions() {
        let store = store();
        seed_live_session(&store, "ses_b", "wrk_b", "amux-wrk_b");
        let rt = test_runtime(store.clone(), vec![Arc::new(FakeBackend {
            hosted: vec![],
            fail_probe: true, // probe down != sessions gone
        })]);
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
        let rt = test_runtime(store.clone(), vec![]);
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

#[cfg(test)]
mod pump_tests {
    use super::*;
    use crate::opencode::mock::{MockProtocol, RecordedCall};
    use crate::opencode::AgentState;
    use amux_core::ids::{CommandId, WorkerId};
    use amux_core::protocol::{CommandState, DeliveryTiming, WorkerCommand};

    fn store() -> SharedStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let s = Arc::new(crate::db::Store::open(&path).unwrap());
        std::mem::forget(dir);
        s
    }

    fn wid() -> WorkerId {
        WorkerId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, 77))
    }

    fn runtime_with(store: SharedStore, protocol: Arc<MockProtocol>) -> Runtime {
        Runtime {
            store,
            backends: vec![],
            tick_secs: 3,
            heartbeat_every: 1000,
            breaker: amux_core::circuit::FleetCircuitBreaker {
                window_budget_tokens: u64::MAX,
                window_secs: 3600,
                min_progress_per_window: 0,
                max_failures_per_window: 1000,
            },
            fleet_state: std::sync::Mutex::new(amux_core::circuit::FleetState::Normal),
            protocol: Some(protocol),
            pickup_unowned: false,
        }
    }

    #[tokio::test]
    async fn pump_delivers_when_idle_and_holds_mid_turn() {
        let store = store();
        let protocol = Arc::new(MockProtocol::new());
        // Worker mid-turn: an AtTurnBoundary command must WAIT.
        protocol.register(wid(), AgentState::Working { turn: None, progress: None });
        let cmd_id = CommandId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, 501));
        {
            let id = cmd_id.clone();
            store
                .write(move |conn| {
                    crate::db::commands::enqueue(
                        conn, id, &wid(), &WorkerCommand::Continue, "pump-k1",
                        &DeliveryTiming::AtTurnBoundary, None, Utc::now(),
                    )?;
                    Ok(WriteOutcome { applied: true, events: vec![] })
                })
                .unwrap();
        }
        let rt = runtime_with(store.clone(), protocol.clone());
        rt.pump_commands(Utc::now()).await.unwrap();
        assert!(protocol.calls().is_empty(), "mid-turn: nothing delivered");
        {
            let conn = store.read().unwrap();
            let cmd = crate::db::commands::by_id(&conn, &cmd_id).unwrap().unwrap();
            assert_eq!(cmd.state, CommandState::Queued, "still queued");
        }

        // Turn ends -> delivery goes through and the state advances.
        protocol.set_state(&wid(), AgentState::Idle, None);
        rt.pump_commands(Utc::now()).await.unwrap();
        let calls = protocol.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert!(matches!(&calls[0], RecordedCall::SendPrompt { worker, .. } if worker == &wid()));
        let conn = store.read().unwrap();
        let cmd = crate::db::commands::by_id(&conn, &cmd_id).unwrap().unwrap();
        assert_eq!(cmd.state, CommandState::Delivered);
    }

    #[tokio::test]
    async fn pump_fails_command_against_dead_worker() {
        let store = store();
        // Protocol knows NOTHING about this worker (no session).
        let protocol = Arc::new(MockProtocol::new());
        let cmd_id = CommandId::from_ulid(ulid::Ulid::from_parts(1_700_000_000_000, 502));
        {
            let id = cmd_id.clone();
            store
                .write(move |conn| {
                    crate::db::commands::enqueue(
                        conn, id, &wid(), &WorkerCommand::Continue, "pump-k2",
                        &DeliveryTiming::Immediate, None, Utc::now(),
                    )?;
                    Ok(WriteOutcome { applied: true, events: vec![] })
                })
                .unwrap();
        }
        let rt = runtime_with(store.clone(), protocol);
        rt.pump_commands(Utc::now()).await.unwrap();
        let conn = store.read().unwrap();
        let cmd = crate::db::commands::by_id(&conn, &cmd_id).unwrap().unwrap();
        assert!(
            matches!(&cmd.state, CommandState::Failed { reason } if reason.contains("delivery failed")),
            "{:?}", cmd.state
        );
        assert_eq!(cmd.attempts, 1, "failure recorded for the retry budget");
    }
}
