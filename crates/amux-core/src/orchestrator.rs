//! Orchestrator planning core (RR-0029, Invariants 9, 10, 22, 47, 48, 49).
//!
//! Pure: `plan_tick` maps a complete picture of the fleet to a plan —
//! assignments to make, leases to reclaim, stalls to report. The runtime
//! loop in amux-server feeds it real state and executes the plan; the
//! simulation feeds it fake state and asserts on the plan. Same function,
//! which is what makes the simulation evidence rather than theatre
//! (Invariant 22 / ethos rule 7).

use crate::board::{disposition, Task, TaskDisposition};
use crate::circuit::{stall_check_enabled, FleetState};
use crate::ids::{TaskId, WorkerId};
use crate::limits::AttemptRecord;
use crate::stall::{StallReason, StallViolation};
use crate::worker::{Worker, WorkerState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A worker's claim on a task, with an expiry so a dead worker's task
/// returns to the pool without human intervention (Invariant 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub task: TaskId,
    pub worker: WorkerId,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Bumped on every reclaim; a write carrying a stale generation is
    /// recognizably from a dead claimant.
    pub generation: u64,
}

impl Lease {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

/// One unit of "worker, do this task" (Invariant 9: idempotent,
/// at-least-once). `prior_attempts` is the feed-forward channel
/// (Invariant 49): attempt N+1 sees why attempts 1..N failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkAssignment {
    pub task: TaskId,
    pub worker: WorkerId,
    pub attempt: u32,
    pub lease: Lease,
    pub idempotency_key: String,
    pub prior_attempts: Vec<AttemptRecord>,
}

/// Priority inputs the scorer weighs. All optional-with-defaults so callers
/// supply what they know (Invariant 25: hints, not hard scheduling).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PriorityHints {
    /// Explicit user priority (higher = sooner).
    pub explicit: i32,
    /// How many tasks transitively depend on this one (critical path).
    pub dependents: u32,
    /// Tasks this worker touched before score higher on the same worker.
    pub affinity_worker: Option<WorkerId>,
}

/// Everything `plan_tick` looks at. The runtime assembles it from the store;
/// the simulation constructs it directly.
#[derive(Debug, Clone)]
pub struct TickInputs<'a> {
    pub now: DateTime<Utc>,
    pub tasks: &'a [Task],
    pub workers: &'a [Worker],
    pub leases: &'a [Lease],
    pub fleet_state: &'a FleetState,
    pub hints: &'a BTreeMap<TaskId, PriorityHints>,
    /// Attempt history per task, newest last (feed-forward).
    pub attempts: &'a BTreeMap<TaskId, Vec<AttemptRecord>>,
    /// Effective gates for disposition (resolved upstream per scope).
    pub gates: &'a [crate::board::Gate],
    /// How long a fresh lease lives.
    pub lease_secs: i64,
    /// Max concurrent leases per worker (WIP limit).
    pub wip_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickPlan {
    pub assignments: Vec<WorkAssignment>,
    /// Leases past expiry: runtime releases them and bumps generation.
    pub reclaim: Vec<Lease>,
    pub stalls: Vec<StallViolation>,
}

/// Age factor: an hour of waiting outranks one explicit priority point, so
/// starvation self-corrects without a separate aging pass.
fn score(task: &Task, hints: Option<&PriorityHints>, now: DateTime<Utc>) -> i64 {
    let h = hints.cloned().unwrap_or_default();
    let age_hours = (now - task.created_at).num_hours().max(0);
    (h.explicit as i64) * 10 + (h.dependents as i64) * 5 + age_hours
}

fn worker_available(w: &Worker, live_leases: usize, wip_limit: usize) -> bool {
    if live_leases >= wip_limit {
        return false;
    }
    matches!(
        w.state,
        WorkerState::Idle { .. } | WorkerState::Stopped | WorkerState::Starting
    )
}

/// The planning function. Deterministic: same inputs, same plan — ties
/// break on ids, never on iteration order of a hash map.
pub fn plan_tick(inputs: &TickInputs) -> TickPlan {
    let mut plan = TickPlan {
        assignments: vec![],
        reclaim: vec![],
        stalls: vec![],
    };

    // 1. Expired leases release first — their tasks are assignable this
    // same tick (a crashed worker must not cost an extra tick interval).
    let mut live_leases: Vec<&Lease> = vec![];
    for lease in inputs.leases {
        if lease.is_expired(inputs.now) {
            plan.reclaim.push(lease.clone());
        } else {
            live_leases.push(lease);
        }
    }
    let leased_tasks: BTreeSet<&TaskId> = live_leases.iter().map(|l| &l.task).collect();
    let mut lease_count: BTreeMap<&WorkerId, usize> = BTreeMap::new();
    for l in &live_leases {
        *lease_count.entry(&l.worker).or_default() += 1;
    }

    // 2. When the circuit is open the fleet is halted: no assignments, no
    // stall reports (Invariant 10+48 — the stall-fixer must not fight the
    // breaker).
    if !matches!(inputs.fleet_state, FleetState::Normal) {
        return plan;
    }

    // 3. Runnable, unleased tasks, best score first (ties: older id first
    // for determinism).
    let mut candidates: Vec<&Task> = inputs
        .tasks
        .iter()
        .filter(|t| {
            matches!(
                disposition(t, inputs.tasks, inputs.gates),
                TaskDisposition::Runnable
            ) && !leased_tasks.contains(&t.id)
        })
        .collect();
    candidates.sort_by(|a, b| {
        let sa = score(a, inputs.hints.get(&a.id), inputs.now);
        let sb = score(b, inputs.hints.get(&b.id), inputs.now);
        sb.cmp(&sa).then_with(|| a.id.cmp(&b.id))
    });

    // 4. Match to available workers. Owned tasks go only to their owner;
    // unowned tasks go to the best available worker (affinity first).
    let mut assigned_this_tick: BTreeMap<&WorkerId, usize> = BTreeMap::new();
    for task in candidates {
        let capacity = |w: &Worker| {
            let held = lease_count.get(&w.id()).copied().unwrap_or(0)
                + assigned_this_tick.get(&w.id()).copied().unwrap_or(0);
            worker_available(w, held, inputs.wip_limit)
        };
        let chosen: Option<&Worker> = match &task.worker {
            Some(owner) => inputs
                .workers
                .iter()
                .find(|w| w.id() == owner)
                .filter(|w| capacity(w)),
            None => {
                let hint = inputs.hints.get(&task.id);
                let affinity = hint.and_then(|h| h.affinity_worker.as_ref());
                inputs
                    .workers
                    .iter()
                    .filter(|w| capacity(w))
                    .min_by_key(|w| {
                        // Affinity wins; then least-loaded; then id for
                        // determinism.
                        let aff = if Some(w.id()) == affinity { 0 } else { 1 };
                        let load = lease_count.get(&w.id()).copied().unwrap_or(0)
                            + assigned_this_tick.get(&w.id()).copied().unwrap_or(0);
                        (aff, load, w.id().clone())
                    })
            }
        };
        if let Some(worker) = chosen {
            let attempt_history = inputs
                .attempts
                .get(&task.id)
                .cloned()
                .unwrap_or_default();
            let attempt = attempt_history.len() as u32 + 1;
            let lease = Lease {
                task: task.id.clone(),
                worker: worker.id().clone(),
                acquired_at: inputs.now,
                expires_at: inputs.now + chrono::Duration::seconds(inputs.lease_secs),
                generation: 0,
            };
            *assigned_this_tick.entry(worker.id()).or_default() += 1;
            plan.assignments.push(WorkAssignment {
                idempotency_key: format!("{}:{}:{}", task.id, worker.id(), attempt),
                task: task.id.clone(),
                worker: worker.id().clone(),
                attempt,
                lease,
                prior_attempts: attempt_history,
            });
        }
    }

    // 5. Stall check (Invariant 10): an idle worker owning a non-terminal,
    // non-waiting task that this tick did NOT assign is a system failure.
    if stall_check_enabled(inputs.fleet_state) {
        let assigned: BTreeSet<&TaskId> = plan.assignments.iter().map(|a| &a.task).collect();
        for w in inputs.workers {
            let WorkerState::Idle { since } = &w.state else {
                continue;
            };
            for t in inputs.tasks {
                if t.worker.as_ref() != Some(w.id()) || assigned.contains(&t.id) {
                    continue;
                }
                if leased_tasks.contains(&t.id) {
                    continue;
                }
                match disposition(t, inputs.tasks, inputs.gates) {
                    TaskDisposition::Terminal | TaskDisposition::Waiting(_) => {}
                    TaskDisposition::Assigned { .. } => {}
                    TaskDisposition::Runnable => {
                        // Runnable + owner idle + not assigned this tick:
                        // only possible when the owner had no capacity —
                        // report it, because "quietly waiting forever" is
                        // the Python board's L3 failure.
                        plan.stalls.push(StallViolation {
                            worker: w.id().clone(),
                            task: t.id.clone(),
                            status: format!("{:?}", t.status),
                            idle_since: *since,
                            reason: StallReason::WorkerIdle,
                        });
                    }
                }
            }
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{ItemType, Task, TaskStatus};
    use crate::worker::{Worker, WorkerCapabilities, WorkerConfig, WorkerState};
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    fn wid(n: u32) -> WorkerId {
        WorkerId::from_ulid(ulid_n(n))
    }

    fn tid(n: u32) -> TaskId {
        TaskId::from_ulid(ulid_n(n + 10_000))
    }

    fn ulid_n(n: u32) -> ulid::Ulid {
        ulid::Ulid::from_parts(1_700_000_000_000, n as u128)
    }

    fn worker(n: u32, state: WorkerState) -> Worker {
        let mut w = Worker::new(
            wid(n),
            WorkerConfig {
                display_name: format!("w{n}"),
                name_aliases: vec![],
                cwd: "/tmp".into(),
                provider: crate::provider::ProviderId("claude".into()),
                model: None,
                backend: crate::session::BackendId::herdr(),
                environment: Default::default(),
                permissions: vec![],
                group: None,
            },
            WorkerCapabilities::default(),
        );
        w.state = state;
        w
    }

    fn task(n: u32, status: TaskStatus, owner: Option<u32>) -> Task {
        let mut t = Task::create(
            tid(n),
            format!("task {n}"),
            ItemType::Code,
            crate::events::Actor::System {
                component: "test".into(),
            },
            now() - chrono::Duration::hours(1),
        );
        t.status = status;
        t.worker = owner.map(wid);
        t
    }

    fn inputs<'a>(
        tasks: &'a [Task],
        workers: &'a [Worker],
        leases: &'a [Lease],
        fleet: &'a FleetState,
        hints: &'a BTreeMap<TaskId, PriorityHints>,
        attempts: &'a BTreeMap<TaskId, Vec<AttemptRecord>>,
    ) -> TickInputs<'a> {
        TickInputs {
            now: now(),
            tasks,
            workers,
            leases,
            fleet_state: fleet,
            hints,
            attempts,
            gates: &[],
            lease_secs: 600,
            wip_limit: 1,
        }
    }

    #[test]
    fn assigns_runnable_task_to_idle_owner() {
        let tasks = vec![task(1, TaskStatus::Todo, Some(1))];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let plan = plan_tick(&inputs(&tasks, &workers, &[], &FleetState::Normal, &h, &a));
        assert_eq!(plan.assignments.len(), 1);
        assert_eq!(plan.assignments[0].worker, wid(1));
        assert_eq!(plan.assignments[0].attempt, 1);
        assert!(plan.stalls.is_empty(), "assigned means not stalled");
    }

    #[test]
    fn wip_limit_prevents_double_assignment() {
        let tasks = vec![
            task(1, TaskStatus::Todo, Some(1)),
            task(2, TaskStatus::Todo, Some(1)),
        ];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let plan = plan_tick(&inputs(&tasks, &workers, &[], &FleetState::Normal, &h, &a));
        assert_eq!(plan.assignments.len(), 1, "wip_limit 1 caps at one");
        // The second runnable task correctly reports a stall (owner is
        // saturated, task sits) — visible, not silent (L3).
        assert_eq!(plan.stalls.len(), 1);
    }

    #[test]
    fn expired_lease_reclaimed_and_task_reassigned_same_tick() {
        let tasks = vec![task(1, TaskStatus::Todo, Some(1))];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let leases = vec![Lease {
            task: tid(1),
            worker: wid(2),
            acquired_at: now() - chrono::Duration::hours(2),
            expires_at: now() - chrono::Duration::hours(1),
            generation: 3,
        }];
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let plan = plan_tick(&inputs(&tasks, &workers, &leases, &FleetState::Normal, &h, &a));
        assert_eq!(plan.reclaim.len(), 1);
        assert_eq!(plan.assignments.len(), 1, "reclaim frees the task this tick");
    }

    #[test]
    fn live_lease_blocks_reassignment() {
        let tasks = vec![task(1, TaskStatus::Todo, Some(1))];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let leases = vec![Lease {
            task: tid(1),
            worker: wid(1),
            acquired_at: now(),
            expires_at: now() + chrono::Duration::hours(1),
            generation: 0,
        }];
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let plan = plan_tick(&inputs(&tasks, &workers, &leases, &FleetState::Normal, &h, &a));
        assert!(plan.assignments.is_empty());
        assert!(plan.reclaim.is_empty());
    }

    #[test]
    fn circuit_open_halts_assignment_and_stall_reporting() {
        let tasks = vec![task(1, TaskStatus::Todo, Some(1))];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let open = FleetState::CircuitOpen {
            reason: crate::circuit::CircuitOpenReason::ManualStop,
            opened_at: now(),
        };
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let plan = plan_tick(&inputs(&tasks, &workers, &[], &open, &h, &a));
        assert!(plan.assignments.is_empty());
        assert!(plan.stalls.is_empty());
    }

    #[test]
    fn priority_scoring_orders_assignments() {
        // One worker, wip 1: only the highest-scored task gets assigned.
        let tasks = vec![
            task(1, TaskStatus::Todo, None),
            task(2, TaskStatus::Todo, None),
        ];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let mut hints = BTreeMap::new();
        hints.insert(
            tid(2),
            PriorityHints {
                explicit: 5,
                ..Default::default()
            },
        );
        let a = BTreeMap::new();
        let plan = plan_tick(&inputs(&tasks, &workers, &[], &FleetState::Normal, &hints, &a));
        assert_eq!(plan.assignments.len(), 1);
        assert_eq!(plan.assignments[0].task, tid(2), "explicit priority wins");
    }

    #[test]
    fn prior_attempts_feed_forward() {
        let tasks = vec![task(1, TaskStatus::Todo, Some(1))];
        let workers = vec![worker(1, WorkerState::Idle { since: now() })];
        let mut attempts = BTreeMap::new();
        attempts.insert(
            tid(1),
            vec![AttemptRecord {
                attempt: 1,
                failure_reason: "tests failed: assertion x".into(),
                rejected_evidence: vec![],
                tokens_spent: 1000,
                wall_clock_secs: 60,
                decomposition_attempted: false,
                tree_status: None,
                at: now() - chrono::Duration::hours(1),
            }],
        );
        let h = BTreeMap::new();
        let plan = plan_tick(&inputs(&tasks, &workers, &[], &FleetState::Normal, &h, &attempts));
        assert_eq!(plan.assignments[0].attempt, 2);
        assert_eq!(plan.assignments[0].prior_attempts.len(), 1);
        assert!(plan.assignments[0].prior_attempts[0]
            .failure_reason
            .contains("assertion x"));
    }

    /// RR-0029's simulation requirement: 50 workers / 200 tasks, no task
    /// double-assigned, every assignment to an available worker, plan is
    /// deterministic across runs.
    #[test]
    fn simulation_50_workers_200_tasks() {
        let workers: Vec<Worker> = (0..50)
            .map(|n| worker(n, WorkerState::Idle { since: now() }))
            .collect();
        let tasks: Vec<Task> = (0..200)
            .map(|n| task(n, TaskStatus::Todo, if n % 3 == 0 { Some(n % 50) } else { None }))
            .collect();
        let (h, a) = (BTreeMap::new(), BTreeMap::new());
        let i = inputs(&tasks, &workers, &[], &FleetState::Normal, &h, &a);
        let plan1 = plan_tick(&i);
        let plan2 = plan_tick(&i);
        assert_eq!(plan1, plan2, "planning must be deterministic");

        // No double-assignment of tasks or over-assignment of workers.
        let mut seen_tasks = BTreeSet::new();
        let mut per_worker: BTreeMap<&WorkerId, usize> = BTreeMap::new();
        for asg in &plan1.assignments {
            assert!(seen_tasks.insert(&asg.task), "task double-assigned");
            *per_worker.entry(&asg.worker).or_default() += 1;
        }
        for (_, count) in per_worker {
            assert!(count <= 1, "wip limit exceeded");
        }
        // 50 idle workers, wip 1 -> exactly 50 assignments.
        assert_eq!(plan1.assignments.len(), 50);
    }
}
