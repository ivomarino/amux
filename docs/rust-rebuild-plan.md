# amux Rust Rebuild Plan

Rewrite of `amux-server.py` (77k-line Python single-file server) in Rust. Not a port
-- a redesign that makes the implicit architecture explicit.

## What exists today

| Layer | Lines | % of file |
|---|---|---|
| Dashboard SPA (inline HTML/CSS/JS) | ~44,300 | 57% |
| Python server (HTTP, API, jobs, integrations) | ~32,700 | 43% |

- **77,050 lines**, single file, hand-rolled `BaseHTTPRequestHandler`
- **47 SQLite tables**, single DB file
- **212 API routes** (250+ method/path combos)
- **~30 background jobs** (scheduler, snapshots, rate-limit watchdog, steering, token ledger, email sync, etc.)
- **3 terminal backends**: herdr (primary process host), tmux, iTerm2
- **Structured agent protocol**: OpenCode (structured commands, events, lifecycle)
- **4 LLM providers**: Claude Code (OAuth + API key), Gemini, Codex, Ollama
- **Full SPA dashboard** with revisioned SSE, delta sync, PWA/offline, dark/light themes

## Why Rust

1. **Real concurrency**: Python's GIL serializes 30+ background jobs on a ThreadingHTTPServer. Rust gives async + multi-threading with no global lock.
2. **Type-enforced invariants**: the new architecture has explicit scopes, typed commands/events, and state machines. A type system makes these compile-time guarantees, not runtime hopes.
3. **Memory**: Python's per-object overhead on a 24/7 desktop server alongside 40+ workers.
4. **Single binary**: one artifact to deploy, no venv or lazy imports.

## The central insight: amux is an orchestrator, not a session manager

The existing system describes itself as a "session manager." That is wrong. amux is an
**orchestrator that uses workers to drive work to completion.** The difference matters
because it determines where every architectural boundary falls.

The Python server has an implicit orchestrator scattered across pickup.rs, advance-nudge,
steering, snapshot, gates, and session startup. Making it explicit is the single biggest
architectural change in this rewrite.

---

## System invariants (define before building)

### Invariant 1: Worker != Session != Backend

A **worker** is a durable amux entity with identity, configuration, capabilities, and
state that survives crashes, context exhaustion, and server restarts.

A **session** is an execution instance: a running process inside a terminal backend,
owned by a worker. A worker may have many sessions over its lifetime (crash -> restart,
context exhaustion -> new session, explicit restart).

A **backend** is the process host: herdr, tmux, native PTY, or anything else. It
spawns, persists, captures, and provides terminal access -- but does not observe or
decide. Structured agent semantics (commands, events, lifecycle state) come from
**OpenCode**, not from the backend.

```
Worker (durable entity)
 ├── Session 1 (ran, hit context limit, ended)
 ├── Session 2 (ran, crashed via OOM, ended)
 └── Session 3 (currently running)
       └── Backend: herdr agent "worker-name"
```

```rust
struct Worker {
    id: WorkerId,
    name: String,
    group: Option<GroupId>,
    config: WorkerConfig,
    capabilities: WorkerCapabilities,
    state: WorkerState,
}

struct Session {
    id: SessionId,
    worker_id: WorkerId,
    backend: BackendKind,
    process: Option<ProcessRef>,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    exit_reason: Option<ExitReason>,
}
```

### Invariant 2: Three-tier scope with deterministic inheritance

Everything configurable lives at one of three scopes: **Global -> Group -> Worker**.
Worker overrides Group overrides Global. This applies uniformly to:

- Environment variables
- Board column definitions and gates
- Memories / instructions
- Model / runtime configuration
- Schedules
- Permissions
- Integrations (MCP servers, tools)
- Automation behavior (auto-compact, auto-restart, pickup)

One resolver, used everywhere:

```rust
enum Scope {
    Global,
    Group(GroupId),
    Worker(WorkerId),
}

fn effective_config<T: Mergeable>(
    global: &T,
    group: Option<&T>,
    worker: &T,
) -> T {
    let mut effective = global.clone();
    if let Some(g) = group {
        effective.merge(g);  // group overrides global
    }
    effective.merge(worker);  // worker overrides group
    effective
}
```

Do NOT implement environment inheritance separately from memory inheritance separately
from gate inheritance. Scope resolution is a primitive.

### Invariant 3: Board is the system of record for all work

The board is not a visualization layer. It is the canonical state of what work exists,
who owns it, and where it is in its lifecycle. Every status transition goes through
the board's transactional state machine. No work happens off-board.

```rust
enum BoardTransition {
    Create { title: String, item_type: ItemType },
    Claim { worker_id: WorkerId, lease: Lease },
    Start,
    Submit,
    RequestReview { reviewer: Actor },
    Approve,
    Reject { reason: String },
    Complete { evidence: Vec<Evidence> },
    Verify { criteria: Vec<Criterion>, evidence: Vec<Evidence> },
    Force { status: Status, reason: String },
    Archive { reason: String },
    Restore { reason: String },
}

// Every transition: one function, one code path, audited by construction
fn apply_transition(
    item: &Issue,
    tx: BoardTransition,
    actor: &Actor,
    scope: &EffectiveConfig,  // gates come from scope
) -> Result<Issue, GateError>;
```

Gates are scoped: global gates apply to all groups, group gates override for that
group, worker-level gates can further specialize. A group might require code review
while another group does not.

### Invariant 4: Board issues form a dependency graph

Issues have typed relations:

```rust
enum IssueRelation {
    Blocks,
    DependsOn,
    Parent,
    Child,
    Related,
    Verifies,
}
```

"Runnable" is derived centrally from the graph:

```
A ──┬──> C
B ──┘
A and B can run concurrently; C cannot start until both are done.
```

The orchestrator uses this graph to determine what to assign, not a flat queue scan.

### Invariant 5: Typed command/event protocol (the D1 exit)

The terminal is an adapter, not the control plane. The system speaks typed commands
and events internally via **OpenCode's structured agent protocol**. Prompts,
messages, cancellation, and lifecycle queries go through OpenCode directly --
never routed through the backend. The backend (herdr, tmux, or native PTY)
handles only process hosting: start, stop, inspect. This separation eliminates
most scraping entirely.

```rust
enum WorkerCommand {
    ExecuteIssue(IssueId),
    Continue,
    DeliverMessage(MessageId),
    Verify(IssueId),
    Review(IssueId),
    Cancel,      // DeliveryTiming::Immediate
    Pause,       // DeliveryTiming::Immediate
    Resume,      // DeliveryTiming::Immediate
}

enum WorkerEvent {
    Started,
    TurnStarted { turn_id: TurnId },
    Progress(ProgressReport),
    Waiting(WaitReason),
    ToolUsed(ToolEvent),
    IssueUpdated(IssueId),
    TurnCompleted(TurnResult),
    RateLimited(RateLimit),
    ContextLow(u8),
    Failed(Failure),
    Exited(ExitStatus),
}
```

When Claude Code hooks fire, they emit `WorkerEvent` variants directly. When hooks
don't cover something (rate limits today), the terminal scraper infers a `WorkerEvent`
from the captured text. The consumer never knows which source produced the event -- it
just processes `WorkerEvent`s. As Claude Code's hook coverage grows, scrapers shrink
to liveness checks.

This is the actual D1 exit. Backend scraping becomes a fallback adapter that emits
`WorkerEvent::RateLimited` instead of the orchestrator matching regexes. OpenCode's
structured agent protocol handles most transitions directly; scraping remains only
for provider-specific signals (rate-limit messages, context warnings) that neither
OpenCode nor provider hooks expose.

### Invariant 6: Turn is a first-class concept

A turn is one cycle of a worker's execution: it starts when the worker begins
processing, ends when it yields (waiting for input, rate-limited, idle, done).

```rust
struct Turn {
    id: TurnId,
    session_id: SessionId,
    issue_id: Option<IssueId>,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    outcome: Option<TurnOutcome>,
    events: Vec<WorkerEvent>,
}
```

Turn boundaries are where:
- Steering messages are delivered
- Board consequences are evaluated
- Memory is refreshed
- State snapshots are taken
- Orchestrator decisions happen

Without an explicit turn, "turn boundary" is a collection of heuristics.

### Invariant 7: Done != Verified

Done is a worker's claim. Verified is the harness's conclusion.

```rust
struct Verification {
    issue_id: IssueId,
    verifier: Actor,
    criteria: Vec<Criterion>,
    evidence: Vec<Evidence>,
    result: VerificationResult,
    verified_at: DateTime<Utc>,
}

enum VerificationResult {
    Passed,
    Failed { reason: String },  // -> issue returns to InProgress
}
```

An issue moves `done -> verified` only when:
1. CI/CD green on the merged commit
2. Deployed to prod
3. Confirmed working in prod (Playwright, smoke, manual)
4. Zero regressions

This is what distinguishes amux from "workers with a kanban board."

### Invariant 8: Provider agnosticism

amux orchestrates work. The model runtime is pluggable:

```rust
enum Provider {
    ClaudeCode { auth: ClaudeAuth },
    Gemini { api_key: String },
    Codex { auth: CodexAuth },
    Ollama { model: String, endpoint: Url },
}

struct WorkerConfig {
    provider: Provider,
    model: Option<String>,
    #[serde(default)]  // default: Herdr
    backend: BackendKind,
    // ...
}
```

Every provider needs:
- A way to start a session (CLI invocation differs per provider)
- A way to detect its rate-limit patterns (different regexes per provider)
- A way to detect its prompt/idle state

Provider-specific logic lives in provider modules, not in `if provider == "gemini"`
branches scattered everywhere.

### Invariant 9: Idempotent + at-least-once for all orchestration

Rust's concurrency makes latent races easier to trigger. Every consequential
operation gets an idempotency key.

```rust
struct WorkAssignment {
    issue_id: IssueId,
    worker_id: WorkerId,
    attempt: u32,
    lease: Lease,
    context: WorkContext,
    idempotency_key: Uuid,
}

struct Lease {
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    generation: u64,
}
```

Claiming is atomic: `UPDATE issues SET status='claimed', worker_id=?, generation=?
WHERE id=? AND status='todo' AND generation=?`. Exactly one claimant.

Dead workers: lease expires -> issue becomes runnable again. No manual intervention.

Startup reconciliation:

```rust
async fn reconcile_on_startup(ctx: &AppContext) {
    // DB says running + backend says missing -> mark interrupted, restart
    // Backend exists + DB says stopped -> adopt or kill
    // Issue claimed + lease expired -> release, requeue
    // Pending steering message -> redeliver
    // Schedule fire persisted but execution missing -> retry
}
```

### Invariant 10: No-stall guarantee (the cardinal acceptance criterion)

**If a worker is idle and any of its issues are not in a terminal state, that is a
system failure.** Terminal states are: `verified`, `archived`, `discarded`, and
`blocked_by_user`. Everything else must keep moving.

**Every non-terminal issue must have exactly one of: a runnable next action, a named
actor responsible for the next action, or a structured wait reason.** "Nothing is
driving this" is an impossible state, not a thing the stall detector discovers
afterward.

```rust
enum TerminalStatus {
    Verified,
    Archived,
    Discarded,
    BlockedByUser { reason: String },
}

enum WaitingFor {
    Dependency(IssueId),
    Gate { gate: GateId, missing: Vec<GateCriterion> },
    User { actor: Actor, question: String },
    Provider { kind: WaitReason },
    ExternalCondition { description: String, check: Option<VerifierKind> },
    Capability { needed: Vec<String>, available_workers: Vec<WorkerId> },
}

// Every non-terminal issue resolves to exactly one of these:
enum IssueDisposition {
    Runnable,                            // can be picked up now
    Assigned { worker: WorkerId },       // someone is working on it
    Waiting(WaitingFor),                 // blocked, with structured reason
    Terminal(TerminalStatus),            // done, nothing to do
}

fn disposition(issue: &Issue, board: &Board) -> IssueDisposition {
    // This function must be total -- every issue resolves to one variant.
    // If none of the conditions match, that is a compile-time error
    // (exhaustive match), not a runtime discovery.
}

// The orchestrator runs this check on every tick:
fn stall_check(worker: &Worker, board: &Board) -> Vec<StallViolation> {
    if worker.state != WorkerState::Idle { return vec![]; }
    board.issues_for_worker(worker.id)
        .filter(|i| matches!(disposition(i, board), IssueDisposition::Runnable))
        .map(|i| StallViolation {
            worker_id: worker.id,
            issue_id: i.id,
            status: i.status,
            idle_since: worker.idle_since,
        })
        .collect()
}
```

When the orchestrator detects a stall:
1. If the worker is rate-limited: wait (not idle, not a stall)
2. If the issue is blocked by a dependency: no stall (it is `Waiting(Dependency(...))`)
3. If the issue is waiting on a gate: no stall (it is `Waiting(Gate(...))`)
4. If the worker has no runnable issues left in its scope: escalate to the group or
   reassign
5. Otherwise: the worker MUST be given the issue and told to continue

The dashboard shows `WaitingFor` inline on every non-terminal, non-assigned issue.
A user looking at the board sees exactly WHY each item is waiting, not just that it
is stuck. `IssueDisposition::Waiting` with no resolution path (e.g., waiting on a
capability no worker has) triggers an escalation alert.

This is tested in every Playwright golden scenario: at the end of every test, assert
that no worker is idle with runnable issues. A stall is a CI failure.

### Invariant 11: Worker state is always current

The current system's worker status (idle, working, rate-limited, etc.) is frequently
stale because it depends on polling terminal output. The Rust system treats stale
status as a bug.

Worker state transitions are event-driven, not poll-derived:

```rust
// Every WorkerEvent updates state immediately
fn process_event(worker: &mut Worker, event: WorkerEvent) {
    match event {
        WorkerEvent::TurnStarted { .. } => worker.state = WorkerState::Active,
        WorkerEvent::TurnCompleted(_) => worker.state = WorkerState::Idle { since: now() },
        WorkerEvent::Waiting(reason) => worker.state = WorkerState::Waiting(reason),
        WorkerEvent::RateLimited(rl) => worker.state = WorkerState::RateLimited {
            kind: rl.kind,
            reset_at: rl.reset_at,
            provider: rl.provider,
        },
        WorkerEvent::ContextLow(pct) => worker.context_pct = Some(pct),
        WorkerEvent::Failed(_) => worker.state = WorkerState::Error,
        WorkerEvent::Exited(_) => worker.state = WorkerState::Stopped,
        _ => {}
    }
    // DB write increments global rev; SSE carries the revisioned StateEvent
    // Dashboard applies only if rev > local rev (Invariant 35)
    db.persist_worker_state(worker)?;
    emit_state_event(worker.id, EntityType::Worker, Mutation::StateChanged);
}
```

Event sources, in priority order:
1. **OpenCode structured protocol**: typed agent lifecycle events (turn start/end,
   waiting, completion, context state) reported directly as WorkerEvents
2. **Provider hooks** (Claude Code): events in real time via Stop/UserPromptSubmit
3. **Terminal adapter** (fallback): polls and translates rendered terminal output to
   WorkerEvents for provider-specific signals OpenCode/hooks cannot expose

The consumer code is identical regardless of source. OpenCode handles structured
lifecycle transitions for all providers; the terminal adapter handles only
provider-specific rate-limit patterns that no structured protocol exposes yet.

**Per-provider event coverage** (what each source can report):

| Event | OpenCode | Claude (hooks) | Terminal scrape |
|---|---|---|---|
| TurnStarted | structured event | UserPromptSubmit hook | regex (fallback) |
| TurnCompleted | structured event | Stop hook | regex (fallback) |
| Waiting/Blocked | structured event | -- | regex (fallback) |
| RateLimited | -- | -- | provider-specific regexes |
| ContextLow | structured event | -- | regex (fallback) |
| Failed (crash) | structured event | -- | process check (fallback) |

OpenCode provides lifecycle events for all providers. Provider hooks complement
for provider-specific signals. Terminal scraping is the fallback for signals
neither OpenCode nor hooks cover (primarily rate-limit patterns).

**Acceptance test**: dashboard shows correct worker status within 2s of every state
change. Tested for all 4 providers.

### Invariant 12: Groups are first-class (not tags)

Groups replace the tag-based isolation system. A group is a structural boundary, not
a label.

```rust
struct Group {
    id: GroupId,
    name: String,
    config: GroupConfig,  // overrides global
    gates: Vec<GateDefinition>,  // board column gates for this group
    columns: Vec<ColumnDefinition>,  // board columns (can differ per group)
    members: Vec<WorkerId>,
}
```

Every worker belongs to exactly one group (or the implicit global group). Groups
define their own board column names, column gates, environment, memories, schedules,
and automation behavior. The scope resolver (Invariant 2) makes this uniform.

Workers do NOT use tags for group membership. A worker's `group_id` is a foreign key,
not a string label.

### Invariant 13: API contract is the decoupling layer

The frontend (dashboard SPA) and backend communicate exclusively through a typed API
contract. This enables:
1. Swapping between Python and Rust backends during migration (phase 11)
2. Independent frontend development
3. Third-party integrations against a stable contract

Every API route has a documented contract:

```rust
// Example: POST /api/board
#[derive(Serialize, Deserialize, JsonSchema)]
struct CreateIssueRequest {
    title: String,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default = "default_status")]
    status: Status,
    #[serde(default)]
    item_type: Option<ItemType>,
    #[serde(default)]
    depends_on: Vec<IssueId>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct CreateIssueResponse {
    id: IssueId,
    title: String,
    status: Status,
    item_type: ItemType,
    session: Option<String>,
    // ... all fields
}

// Gate rejection (409):
#[derive(Serialize, Deserialize, JsonSchema)]
struct GateRejection {
    error: String,  // "gate not acknowledged"
    ok: bool,       // false
    blocked: bool,  // true
    gate: Vec<String>,
    attempted_status: Status,
    item: IssueId,
    item_type: ItemType,
    how_to_ack: GateAckInstructions,
    cli: String,    // amux board command to satisfy
}
```

Phase 0 generates an OpenAPI spec from the `JsonSchema` derives. The Playwright tests
validate against the spec. The Python server's responses are validated against the same
spec during shadow mode (phase 11).

### Invariant 14: Offline-first with optimistic sync

The dashboard is an offline-first PWA. All mutations are applied optimistically to
local state, persisted to IndexedDB, and synced to the server when connectivity is
available. This is not a fallback mode -- it is the primary architecture. The current
Python system already has offline queuing (localStorage + IndexedDB), service worker
caching, and an offline banner with manual sync. The Rust rebuild makes this the
foundation instead of a bolt-on.

#### Client-side architecture

```
User action
  -> apply to local state (instant UI update)
  -> persist to IndexedDB (survives tab close)
  -> enqueue sync operation
  -> attempt server sync
     -> success: confirm, reconcile with server state
     -> conflict: show resolution toast, keep local or accept server
     -> offline: queue for retry on reconnect
```

```typescript
// IndexedDB stores (idb-keyval or Dexie):
interface QueuedOperation {
  id: string;              // idempotency key (uuid)
  method: string;          // POST, PATCH, DELETE
  path: string;            // /api/board/AR-7
  body?: any;              // JSON payload
  baseRev: number;         // entity version at time of queue (for conflict detection)
  queuedAt: number;        // timestamp
  retries: number;         // retry count
  lastError?: string;      // last sync error
  optimisticState?: any;   // local state applied before server confirmed
}
```

IndexedDB also persists the `EntityStore` from Invariant 35 (the normalized
client-side cache). This is the offline rendering source. `lastRev` in the
store drives delta sync on reconnect: `GET /api/sync?since_rev={lastRev}`.

#### Server-side support

All server-side sync, conflict detection, and delta reconciliation is defined
in Invariant 35 (server-authoritative revisioned state). The offline layer
uses those primitives:

- **Optimistic writes** include `base_rev` (entity version); conflicts return
  409 with current server state (Invariant 35)
- **Delta sync** on reconnect uses `GET /api/sync?since_rev=N` (Invariant 35)
- **Idempotency keys** on queued operations prevent duplicate application
  (Invariant 9)

#### Conflict resolution

Conflicts are surfaced, never swallowed:

| Conflict type | Resolution |
|---|---|
| Board card moved by someone else while offline | Toast with both states, user picks |
| Board card deleted while offline edit queued | Toast: "card was deleted", discard edit |
| Worker command sent while worker stopped | Toast: "worker stopped", offer restart |
| Entity version conflict (409) | Show server state, user picks or merges |

#### Service worker caching

The SW caches the app shell (HTML/CSS/JS), icons, manifest, and the last-known
server state. On startup:
1. Serve cached shell immediately (instant paint)
2. Fetch fresh state in background
3. If offline, render from IndexedDB cache
4. Reconnect triggers delta sync, not full reload

Cache invalidation: `APP_VER` stamp in the SW. Server bumps it on deploy. SW
detects new version, fetches new shell, activates on next navigation. The current
Python system requires manually bumping `APP_VER` and `CACHE` in sw.js together --
the Rust build stamps both from `build.rs`.

#### Prefetch for deep offline

The current "Save all workers for offline" button prefetches worker peek/history
for all workers. The Rust version:
- Background sync: if the device has been online for 30s and battery > 20%,
  prefetch worker states incrementally (one per second, not all at once)
- Configurable: user picks which workers to cache for offline
- Storage budget: IndexedDB size limit awareness (show usage, prune old data)

**Test plan** (offline-specific; real-time convergence tests are in Invariant 35):
- Playwright: go offline, create board card, send worker command, go online, verify
  both applied with correct base_rev
- Playwright: go offline, queue 5 commands, go online, verify all 5 replay in order
- Playwright: offline queue + entity version conflict (409) shows toast with both states
- Playwright: dashboard renders all tabs from service worker cache while offline
- Playwright: close tab while offline with queued operations, reopen, verify queue
  survives in IndexedDB and replays on reconnect
- Playwright: service worker update -- deploy new version, verify client picks up
  new shell on next navigation without losing queued operations
- Playwright: deep offline -- prefetch 10 workers, go offline, navigate to each
  worker's detail, verify peek/history renders from cache

### Invariant 15: Three cardinal rules

Elevated from lessons learned to architectural law:

1. **No LLM invocation unless the operation requires semantic judgment.** Title
   derivation, label generation, string formatting, gate evaluation, scope resolution,
   context assembly, dependency resolution -- all deterministic, all free. The token
   cost metric (tokens per verified issue) is a first-class dashboard number alongside
   latency, CPU, and RSS.

2. **No state transition without durable provenance.** Every mutation emits an
   append-only event with actor, timestamp, and cause. Provenance is queryable:
   `amux issue AR-123 history` returns the full chain.

3. **No backend/provider-specific behavior above its adapter boundary.** The
   orchestrator, board, scheduler, and dashboard never know whether herdr, tmux, or
   native PTY is hosting the process, or whether Claude or Ollama is the provider.
   If a feature requires `if backend == "herdr"` or `if provider == "claude"` above
   the adapter, the adapter's interface is wrong.

### Invariant 16: Token budgets are a runtime primitive

Not just a context-assembler concern. Budgets govern context assembly, turn execution,
and issue-level cost tracking.

```rust
struct TokenBudget {
    max_input: u32,
    reserved_output: u32,
    max_per_issue: Option<u64>,
    max_per_turn: Option<u32>,
}

struct ContextFragment {
    source: ContextSource,
    priority: u8,
    estimated_tokens: u32,
    content_hash: Hash,
}
```

Context assembly is deterministic priority order:

`issue + acceptance criteria > immediate dependencies > relevant memory > recent turns > broad history`

Never dump entire issue graphs, logs, memories, or prior transcripts. Summarize/cache
once, reference by ID/hash, hydrate on demand. **Tokens consumed per verified issue**
is a core metric on the dashboard.

### Invariant 17: Structural @worker addressing

Mentions are not prompt syntax. They are durable, addressed intent with delivery
tracking.

```rust
enum ActorRef {
    Worker(WorkerId),
    Group(GroupId),
    Orchestrator,
    User(UserId),
}

struct Mention {
    id: MentionId,
    actor: ActorRef,
    instruction: String,
    state: MentionState,
}

enum MentionState {
    Queued,
    Delivered { at: DateTime<Utc> },
    Acknowledged { at: DateTime<Utc> },
    ActedOn { outcome: String },
}
```

`@worker-3 investigate auth regression` parses into a durable command addressed to
worker-3. Works in issue descriptions, comments, board activity, CLI, and dashboard.
Offline safe (queued -> delivered on reconnect). Crash safe (persisted in DB before
delivery attempt).

### Invariant 18: Gates are first-class entities

Gates are not just scoped definitions enforced by transitions. They are database
entities with APIs, history, versions, and explainability.

```rust
struct Gate {
    id: GateId,
    scope: Scope,
    transition: TransitionSelector,  // e.g., doing -> done
    criteria: Vec<Criterion>,
    evaluator: GateEvaluator,
    required_evidence: Vec<EvidenceType>,
}

enum GateEvaluator {
    Manual,                    // human acknowledges
    Deterministic(CheckFn),    // cargo test, HTTP 200, artifact exists
    Model(ModelJudgment),      // semantic evaluation (last resort)
}
```

The critical query:

```
amux issue AR-123 why-blocked

blocked by gate G-9 (scope: group/engineering)
  criterion: integration tests green
  missing evidence: test_run
  suggested command: cargo test --workspace
  last attempt: 2026-08-07 14:22 — failed (3 tests)
```

No opaque "gate failed."

### Invariant 19: Issue state != Execution state

These are separate concepts that must never bleed into each other.

**Board state** (semantic, user-visible):

```
todo -> claimed -> in_progress -> review -> done -> verified
                                         -> blocked
                                         -> discarded
```

**Execution state** (runtime, system-internal):

```
unassigned -> queued -> leased -> running -> waiting
                                          -> rate_limited
                                          -> retrying
                                          -> crashed
                                          -> completed
```

A rate limit changes execution state, never board state. A backend crash changes
execution state, never board state. Context compaction, session replacement, and
provider failover are all execution-state transitions invisible to the board.

The board shows what the work IS. Execution state shows what the worker is DOING.
The orchestrator bridges them: when execution state reaches `completed`, the board
transition to `done` fires (with evidence). When execution state reaches `crashed`,
the orchestrator retries (new session) without touching the board.

### Invariant 20: Fleet-level provider quota management

Rate limiting is not just detection/recovery. The orchestrator knows provider
capacity BEFORE assignment.

```rust
struct ProviderQuota {
    provider: Provider,
    concurrency_limit: usize,
    active_workers: usize,
    requests_remaining: Option<u64>,
    tokens_remaining: Option<u64>,
    reset_at: Option<DateTime<Utc>>,
    rolling_error_rate: f32,
    state: ProviderState,
}

enum ProviderState {
    Available,
    Degraded { reason: String },
    QuotaExhausted { reset_at: DateTime<Utc> },
    ConcurrencyLimited,
    Unavailable { since: DateTime<Utc> },
    AuthExpired,
}
```

Workers can have fallback chains:

```
preferred: Claude
fallback_1: Codex
fallback_2: Ollama (local, always available)
```

The orchestrator routes work to available providers instead of workers thrashing
against known limits. Distinct failure types get distinct recovery:

| Failure | Recovery |
|---|---|
| `QuotaExhausted` | Wait for reset_at, assign to fallback |
| `ConcurrencyLimited` | Queue, assign when slot opens |
| `Unavailable` | Circuit breaker, exponential backoff + jitter |
| `AuthExpired` | Alert user, block provider until re-auth |
| `NetworkFailure` | Retry with backoff, degrade to local (Ollama) |

### Invariant 21: Backend and provider conformance suites

Every backend implementation passes the exact same test suite without changing tests.
Every provider adapter passes the exact same test suite without changing tests.

```rust
// Backend conformance: process lifecycle ONLY (spawn, stop, inspect).
// Runs against MockBackend, HerdrBackend, TmuxBackend, NativePtyBackend:
mod backend_conformance {
    // Process lifecycle
    async fn test_spawn(backend: &dyn SessionBackend);
    async fn test_terminate(backend: &dyn SessionBackend);
    async fn test_status_running(backend: &dyn SessionBackend);
    async fn test_status_after_terminate(backend: &dyn SessionBackend);
    async fn test_process_crash(backend: &dyn SessionBackend);
    async fn test_backend_daemon_disappears(backend: &dyn SessionBackend);
    async fn test_attach_info(backend: &dyn SessionBackend);

    // Reconciliation
    async fn test_restart_reconciliation(backend: &dyn SessionBackend);
    async fn test_stale_session_reconciliation(backend: &dyn SessionBackend);

    // Scale
    async fn test_concurrent_spawns(backend: &dyn SessionBackend);
    async fn test_40_worker_spawn(backend: &dyn SessionBackend);
}

// Protocol conformance: agent communication (prompts, messages, cancel, state).
// Runs against MockProtocol, OpenCodeProtocol:
mod protocol_conformance {
    async fn test_send_prompt(proto: &dyn AgentProtocol);
    async fn test_deliver_message(proto: &dyn AgentProtocol);
    async fn test_cancel(proto: &dyn AgentProtocol);
    async fn test_pause_resume(proto: &dyn AgentProtocol);
    async fn test_state_query(proto: &dyn AgentProtocol);
    async fn test_event_stream(proto: &dyn AgentProtocol);
    async fn test_command_idempotency(proto: &dyn AgentProtocol);
    async fn test_no_duplicate_delivery_after_restart(proto: &dyn AgentProtocol);
    async fn test_multiline_prompt(proto: &dyn AgentProtocol);
    async fn test_unicode_prompt(proto: &dyn AgentProtocol);
    async fn test_large_prompt(proto: &dyn AgentProtocol);
}
```

**The invariant: no test above the backend/provider layer knows which
backend/provider is running. No test above the protocol layer knows which
agent protocol implementation is in use.**

### Invariant 33: Backend independence

Switching a worker between herdr, tmux, and a future native PTY must not alter its
worker identity, issue lifecycle, messages, context, turns, gates, verification
behavior, scheduling behavior, or observable API semantics.

```rust
// Backend is configuration, not identity
struct WorkerConfig {
    // ...
    backend: BackendKind,  // default: Herdr
    // Changing this field and restarting the worker is a valid operation.
    // Everything above the SessionBackend trait is unchanged.
}

enum BackendKind {
    Herdr,      // default, process hosting + persistence + terminal access
    Tmux,       // fallback, terminal multiplexer
    NativePty,  // future, direct PTY ownership
}
```

The layering:

```
Orchestrator
    |
    +------ AgentProtocol (OpenCode) ------+
    |       prompts, messages, cancel,     |
    |       events, state queries          |
    |                                      |
    +------ SessionBackend ------+         |
            process lifecycle    |         |
            (spawn, stop,        |         |
             inspect only)       |         |
            |                    |         |
            +-- HerdrBackend    [default]  |
            +-- TmuxBackend     [fallback] |
            +-- NativePtyBackend [future]  |
                                           v
                                    WorkerEvent stream
```

OpenCode and SessionBackend are independent axes with non-overlapping
responsibilities. OpenCode carries all agent communication: prompts, messages,
cancellation, lifecycle queries, and events. SessionBackend carries only
process lifecycle: spawn, terminate, inspect, reconcile. The orchestrator
never sends a prompt through the backend.

When OpenCode is unavailable (e.g., a provider that does not support it yet),
the terminal adapter falls back to scraping for state and to `send-keys`-style
input -- but this is the degraded path, not the design center.

Rate limits, turns, compaction, messages, issue state, gates, verification,
scheduling, and worker state are completely backend-independent. A worker can
restart on a different backend while preserving all durable amux state. Backend
choice does not require DB/schema changes and does not change worker identity or
issue ownership.

### Invariant 34: Explicit queue semantics

Every queue in the system has a defined delivery contract. The contract specifies
persistence, ordering, dedup, delivery confirmation, retry policy, and dead-letter
behavior. This is especially important with herdr as the default backend: the queue
between the orchestrator and the backend is the critical delivery path, and its
semantics must be testable independently of any backend implementation.

#### The five queues

```rust
struct CommandQueue {
    worker_id: WorkerId,
    commands: VecDeque<QueuedCommand>,
    capacity: usize,               // configurable, default 16
    overflow: OverflowPolicy,      // Reject429
}

struct QueuedCommand {
    id: CommandId,
    command: WorkerCommand,
    idempotency_key: Uuid,
    enqueued_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    precondition: Option<CommandPrecondition>,
    deliver_at: DeliveryTiming,    // Immediate | AtTurnBoundary | After(Duration)
    attempts: u32,
    max_attempts: u32,             // default 3
    state: CommandState,
}

enum CommandState {
    Queued,
    Dispatched { at: DateTime<Utc>, backend_ack: bool },
    Delivered { at: DateTime<Utc> },
    Confirmed { at: DateTime<Utc>, outcome: CommandOutcome },
    Failed { at: DateTime<Utc>, reason: String },
    DeadLettered { at: DateTime<Utc>, reason: String },
}
```

| Queue | Persistence | Ordering | Overflow | Retry | Dead letter |
|---|---|---|---|---|---|
| **Command queue** (per worker) | DB-backed, survives restart | FIFO within priority | Reject 429 at capacity | 3 attempts, backoff | Dead-letter after max attempts, alert |
| **Event channel** (per worker) | In-memory, lossy | Causal (per worker) | Drop oldest, gap marker | No retry (events are facts) | N/A |
| **SSE channel** (per subscriber) | In-memory, lossy | Global rev ordered (Invariant 35) | Drop oldest + reconnect hint | Client detects rev gap, delta syncs | N/A |
| **DB write queue** | In-memory, bounded | Serialized (single writer) | Backpressure -> 503 | Retry on SQLITE_BUSY (3x) | Fail request |
| **Message queue** (Invariant 29) | DB-backed, durable | FIFO per thread | No bound (durable) | Deliver at next turn boundary | Never (messages are durable) |

#### Command delivery contract

The command queue is DB-backed because commands must survive server restarts (the
Python system lost pending steering messages on every restart). The delivery
protocol:

```
1. API/orchestrator enqueues command (persisted to DB, idempotency key recorded)
2. Orchestrator tick: pick next command for each idle/waiting worker
3. Dispatch through AgentProtocol (OpenCode) -- direct to agent, not through backend
4. OpenCode acknowledges receipt (structured response)
5. WorkerEvent confirms execution (worker acted on it)
6. Command marked Confirmed with outcome

If step 3 fails: check process liveness via SessionBackend.status()
If step 4 times out: retry with backoff (OpenCode process may be busy)
If step 5 never arrives: after timeout, mark DeadLettered + StallViolation
```

Duplicate commands with the same idempotency key return the existing result without
re-dispatching. This is critical during restarts: the reconciliation loop
(Invariant 9) reprocesses pending commands, and idempotency prevents double delivery.

#### Command preconditions (freshness at delivery)

A queued command may have been valid at creation time but false at delivery time.
From the commit history: notifications keyed off current state instead of the
assignment event; nudges told workers to act on work whose state had since changed;
steering re-checked whether a board card still existed before delivery.

The fix: automated commands that depend on state carry preconditions and are
revalidated at delivery time.

```rust
enum CommandPrecondition {
    EntityVersion { entity_type: EntityType, entity_id: EntityId, version: u64 },
    EntityStatus { entity_type: EntityType, entity_id: EntityId, status: Status },
    And(Vec<CommandPrecondition>),
}
```

At delivery, the orchestrator evaluates the precondition against current state.
If it fails:

```rust
enum PreconditionResult {
    Satisfied,
    Failed { expected: String, actual: String },
    EntityGone,
}

// Failed precondition -> command expires, never delivered
CommandState::Expired { reason: PreconditionResult }
```

Human-authored messages (user sends text to a worker) carry no precondition and
always deliver. State assertions and automation commands (nudges, advance,
reassignment) carry preconditions. This is the root fix for an entire family of
stale-nudge bugs.

Example:
```
"Tell worker to review AR-42"
precondition: AR-42.version == 17 AND AR-42.status == review

At delivery, AR-42 is now verified:
  -> CommandState::Expired { PreconditionResult::Failed }
  -> not delivered, DurableEvent emitted
```

#### Event ordering guarantees

WorkerEvents are causally ordered per worker but NOT globally ordered across workers.
This matches reality: worker A's TurnCompleted and worker B's TurnStarted have no
causal relationship.

```rust
struct WorkerEvent {
    // ...
    sequence: u64,       // monotonic per worker, for gap detection
    worker_id: WorkerId, // causal ordering is per-worker
}
```

The event channel uses sequence numbers so consumers detect gaps. A gap means
events were dropped under backpressure; the consumer must re-read current state
from the DB rather than inferring it from the event stream.

#### Delivery timing

Not all commands should fire immediately:

```rust
enum DeliveryTiming {
    Immediate,
    AtTurnBoundary,     // steering messages, memory refresh
    After(Duration),    // delayed retry, scheduled commands
    WhenIdle,           // queue until worker finishes current turn
}
```

`AtTurnBoundary` is where messages (Invariant 29) and context refresh are delivered.
`WhenIdle` prevents interrupting a worker mid-turn with a lower-priority command.
`Immediate` is for Cancel, Pause, and Resume -- they override turn boundaries.

#### Dead-letter and observability

Commands that exhaust retries become dead letters. A dead letter is a system failure
(something the orchestrator wanted to happen did not happen) and produces:

1. A `DurableEvent::CommandDeadLettered` with the full command and failure reason
2. A `StallViolation` if the command was issue-related (Invariant 10)
3. A dashboard alert on the worker card

Dead letters are queryable: `GET /api/workers/:id/dead-letters`. The dashboard shows
a count badge on the worker card when dead letters exist. This replaces the Python
system's silent failure mode where steering messages vanished on restart with no trace.

#### Queue depth as a health signal

```rust
struct QueueHealth {
    worker_id: WorkerId,
    depth: usize,
    oldest_command_age: Duration,
    dead_letter_count: u32,
    delivery_rate: f32,     // commands confirmed / commands enqueued, trailing 1h
}
```

A delivery rate below 90% or an oldest command older than 60s triggers a dashboard
warning. The orchestrator uses queue depth as an input to work assignment: a worker
with a deep queue is not a good candidate for new work.

### Invariant 35: Server-authoritative revisioned state

**The backend database is authoritative. The UI may optimistically predict future
state, but every displayed entity must eventually converge to an explicitly
revisioned backend state. Missing, duplicate, reordered, or stale realtime events
must never produce persistent UI divergence.**

SSE is notification, not the source of truth. The UI never infers "latest" from
whichever SSE message it happened to receive last.

#### Global revision

Every mutating DB transaction increments a monotonic global revision. The revision
is the single source of ordering for all state changes across all entity types.

```rust
struct StateRevision {
    rev: u64,                  // monotonically increasing, never reset
    entity_type: EntityType,   // Worker, Issue, Message, Group, Gate, Session, ...
    entity_id: EntityId,
    mutation: Mutation,        // what changed
    at: DateTime<Utc>,
}

// DB: single row table, updated in the same transaction as the mutation
// CREATE TABLE global_rev (id INTEGER PRIMARY KEY CHECK(id = 1), rev INTEGER NOT NULL);
// Every mutating transaction: UPDATE global_rev SET rev = rev + 1 RETURNING rev;
```

#### Entity versions

Each entity carries its own version in addition to the global revision. Global
`rev` answers "what is the latest system state?" Entity `version` answers "is
this exact entity stale?"

```rust
struct Issue {
    id: IssueId,
    version: u64,              // incremented on every mutation to THIS issue
    // ...
}

struct Worker {
    id: WorkerId,
    version: u64,
    // ...
}
```

Concurrency checks use entity version. Global revision drives sync ordering.

#### Event publishing

Every DB mutation publishes a revisioned event:

```
DB mutation
   ↓
commit transaction (atomically increments global rev)
   ↓
publish event { rev, entity_type, entity_id, mutation }
   ↓
UI applies only if rev > local rev
```

```rust
struct StateEvent {
    rev: u64,
    entity_type: EntityType,
    entity_id: EntityId,
    mutation: Mutation,
}
```

SSE carries `StateEvent`s. The client applies them in revision order:

```typescript
function onStateEvent(event: StateEvent) {
    if (event.rev <= state.lastRev) return;           // stale, ignore

    if (event.rev !== state.lastRev + 1) {
        reconcileFrom(state.lastRev);                 // gap detected, delta sync
        return;
    }

    applyMutation(event);
    state.lastRev = event.rev;
}
```

#### Delta sync endpoint

On initial load, reconnect, tab wake, browser `online`, or detected revision gap:

```
GET /api/sync?since_rev=104
```

```rust
#[derive(Serialize)]
struct SyncResponse {
    rev: u64,                              // current global revision
    changes: Vec<StateEvent>,             // all mutations since since_rev
    full_sync_required: bool,             // true if since_rev is too old (pruned)
}
```

The server retains a bounded changelog (configurable, default 10,000 revisions).
If `since_rev` is older than the oldest retained revision, `full_sync_required`
is true and the client does a full state load instead of a delta.

This handles: dropped SSE connections, laptop sleep, flaky Wi-Fi, browser
throttling, server restart, SSE backpressure drops.

#### Optimistic writes with conflict detection

Mutating API calls include the base revision for conflict detection:

```rust
#[derive(Deserialize)]
struct MutationRequest<T> {
    #[serde(flatten)]
    payload: T,
    base_rev: u64,                        // client's last known entity version
    idempotency_key: Option<Uuid>,
}
```

The backend either commits and returns the new revision:

```json
{ "rev": 108, "version": 18, "entity": { ... } }
```

Or rejects as stale:

```
409 Conflict
{ "server_rev": 107, "server_version": 17, "current": { ... } }
```

The client reconciles on conflict. Offline/slow clients cannot silently overwrite
newer state.

#### Normalized client-side state store

One canonical local entity cache. Every screen reads from the same entities:

```typescript
interface EntityStore {
    workers: Map<WorkerId, Worker>;
    issues: Map<IssueId, Issue>;
    groups: Map<GroupId, Group>;
    messages: Map<MessageId, Message>;
    gates: Map<GateId, Gate>;
    sessions: Map<SessionId, Session>;
    lastRev: number;
}
```

The board does not maintain one copy of an issue while the issue-detail modal has
another. Views are projections over the store, not independent state. A mutation
from any source (SSE event, API response, optimistic write) updates the store
once; every view re-renders from the same data.

#### Connection state indicator

The UI shows connection state, subtle but always visible:

```
LIVE · rev 18291
SYNCING…
OFFLINE · last synced 2m ago
STALE · reconnecting
```

Transitions:

| From | To | Trigger |
|---|---|---|
| LIVE | STALE | SSE silence > 18s (existing ping timeout) |
| LIVE | OFFLINE | browser `offline` event |
| STALE | SYNCING | reconnect attempt starts |
| OFFLINE | SYNCING | browser `online` event |
| SYNCING | LIVE | delta sync completes, SSE reconnected |
| SYNCING | STALE | delta sync fails, retrying |

#### Reconciliation triggers

Delta sync fires on all of these (not just SSE reconnect):

1. Initial page load
2. SSE reconnect after drop
3. Tab wake (`visibilitychange` visible)
4. Browser `online` event
5. `pageshow` / `focus` events
6. Revision gap detected in SSE stream
7. Periodic heartbeat (every 60s while LIVE, as a safety net)
8. After any 409 Conflict response

#### E2E test plan

The ugly cases, tested explicitly:

- Drop every 5th SSE event -> UI converges to correct state
- Deliver events out of order -> UI stays correct (revision ordering)
- Duplicate events -> no duplicate effects (idempotent apply)
- Kill/restart server -> UI reconnects and catches up via delta sync
- Sleep browser for 10 minutes -> wakes and delta-syncs to current
- Two tabs mutate same issue -> both converge to same state
- Offline mutation conflicts with newer backend state -> explicit 409, toast
- 1,000 rapid board mutations -> UI finishes at exactly backend rev/state
- SSE backpressure drops 50 events -> gap detection triggers reconcile
- Server changelog pruned (since_rev too old) -> full sync, not partial
- Entity version conflict (two clients edit same issue) -> loser gets 409
- Optimistic write applied then server rejects -> rollback visible to user

### Invariant 36: Single source of truth

**Every durable fact has exactly one canonical owner. Everything else is a projection,
cache, index, or derived representation.**

This is the most frequently violated principle in the commit history. The 30-day
audit found it in multiple independent subsystems:

- `acceb79f`: composer draft lived in 3 stores; clearing one let two others resurrect it
- `59a90e9`: browser profile you logged into vs. profile the agent opened were different stores
- `89e7981`: two things wrote spend; the approximate poller overwrote exact proxy metering
- `b63e0e3`: every UI surface independently guessed what kind of message something was
- `04b2dfc`: server and client had different ideas of valid statuses

The canonical owners:

```
Issue state         -> Board (Invariant 3)
Worker state        -> Worker state machine (Invariant 11)
Message             -> Message store (Invariant 29)
Gate definition     -> Gate store (Invariant 18)
Memory              -> MemoryEntry table (Invariant 42)
Provider state      -> Runtime event stream (Invariant 20)
Browser profile     -> BrowserProfile store
Schedule            -> DurableSchedule table
Scope config        -> Scope resolver (Invariant 2)
Search index        -> Derived from source entities (Invariant 32)
UI state            -> Projection of EntityStore (Invariant 35)
```

The dashboard cache is NOT state. `MEMORY.md` is NOT memory. Herdr output is NOT
worker state. Search index is NOT content. A compacted summary is NOT history.

Implementation: every entity store has a single write path. Caches and projections
are regenerated from the canonical source, never written back. When a cache
disagrees with its source, the source wins unconditionally -- there is no merge.

### Invariant 37: Mutation truthfulness

**A successful mutation response states exactly what was applied. Revision increments
iff authoritative state changed. Unknown mutation fields are errors, never silently
ignored.**

From the commit history: PATCHes that returned 200 but applied nothing; fields
silently dropped from requests; `rev` bumped on no-ops (making "did it change?"
unreliable); `ignored_fields` carried in the response body but never read.

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]  // unknown fields are errors, not silent drops
struct IssuePatch {
    status: Option<Status>,
    desc: Option<String>,
    // ...
    base_rev: u64,             // required for optimistic concurrency (Invariant 35)
}

struct MutationResult {
    applied: bool,             // false if the mutation was a no-op
    rev: u64,                  // incremented only if applied == true
    version: u64,              // entity version, incremented only if applied == true
    entity: Issue,             // current state after (possibly no-op) mutation
}
```

A no-op mutation (setting status to its current value) returns `applied: false`
with the current revision unchanged. The client can distinguish "I changed it"
from "it was already there." Entity version and global revision ONLY increment
on actual state changes.

### Invariant 38: Command freshness

**Automated queued commands that depend on state carry preconditions and are
revalidated at delivery time.** Human-authored messages always deliver.

Defined in Invariant 34 (command preconditions section). Elevated to a standalone
invariant because the commit history shows this as a distinct recurring failure
class: notifications keyed off current state instead of the assignment event,
nudges driving work whose state had changed, steering messages arriving for
issues that no longer exist.

The invariant: a command whose precondition fails at delivery time is expired,
not delivered. The expiration is recorded as a `DurableEvent` (Invariant 24).
Blindly redelivering a durable stale instruction makes the system more reliable
at doing the wrong thing.

### Invariant 39: Derived-data direction

**Source -> derived, never reverse. Search indexes, compacted context, generated
memory files, caches, and UI state never write themselves back into their source
without an explicit user mutation.**

```
canonical entity (DB)
      |
      +-> search index (FTS5)
      +-> worker context (assembled)
      +-> compacted summary (lossy)
      +-> MEMORY.md (projected)
      +-> dashboard cache (IndexedDB)
      +-> iCal feed (generated)

NONE of these arrows reverse.
```

A compacted summary may supersede old content for prompt assembly (Invariant 31),
but it never overwrites or becomes the source history. Search results point to
source entities; modifying a search result modifies the entity, not the index.
Memory is read from `MemoryEntry` rows; `MEMORY.md` is a generated projection,
not a source file.

The violation pattern from the commit history:
```
canonical state
     |
generated representation
     |
gets read back as canonical
     |
old/generated data overwrites newer source
```

This invariant makes that arrow structurally impossible: derived representations
have no write path back to their source.

### Invariant 40: Collection completeness

**Every truncated or paginated response declares whether it is complete and
provides `total/returned/cursor` semantics.**

From the commit history: "we fetched 50 and didn't find it" became "it doesn't
exist" because nothing distinguished a complete result from a truncated one.

```rust
struct PagedResponse<T> {
    items: Vec<T>,
    total: usize,              // total matching items (not just this page)
    returned: usize,           // items in this response
    cursor: Option<String>,    // None = this is the last page
    is_complete: bool,         // true iff returned == total
}
```

Every list endpoint uses `PagedResponse`. The dashboard shows "showing N of M"
when truncated. Search results that hit a limit display the limit. API consumers
can always distinguish "empty result" from "result truncated before the item you
wanted."

### Invariant 41: Test oracle correctness

**Tests must prove externally observable outcomes, not implementation activity.**

The commit history shows tests that passed while the system was broken:

- `8bc9eb3`: a shell prompt was output, so the cloud provider smoke test passed
  (the provider was completely broken)
- `7870384`: the echoed prompt contained expected words, so verification passed
  (the worker never executed anything)
- `9cd2892`: "dashboard renders" passed against Chrome's Privacy error interstitial
  (TLS cert was invalid)
- `2cdbd8a`: "Security scan passed" even though the grep itself errored and never
  scanned anything (exit code not checked)

These are oracle correctness problems, not missing-test problems.

Three testing layers beyond example-based Playwright:

**1. Property testing** (`proptest`) for system invariants (already in Invariant 22):
```rust
proptest! {
    fn single_source_of_truth(ops in arb_ops()) {
        // after any sequence of operations, every derived representation
        // matches its canonical source
    }
    fn liveness(ops in arb_ops()) {
        // every non-terminal issue has a runnable action, assigned actor,
        // or structured wait reason
    }
    fn revision_monotonicity(ops in arb_ops()) {
        // global rev never decreases; entity version never decreases
    }
    fn convergence(events in arb_sse_events()) {
        // after applying any permutation of events with gap detection,
        // client state matches server state
    }
}
```

**2. Deterministic orchestrator simulation** (Invariant 22) with fake
time/providers/backends.

**3. Historical incident regression corpus**: encode the last month's incidents
as concrete test cases. Every Rust build proves the architecture cannot reproduce
them.

```rust
mod incident_regression {
    fn incident_2026_07_30_451_fold_card();       // card with 451 tasks
    fn incident_2026_07_30_duplicate_draft();     // draft in 3 stores
    fn incident_2026_07_30_board_read_after_write(); // stale cache
    fn incident_2026_07_31_glyph_mismatch();      // Unicode rate-limit
    fn incident_2026_08_xx_stale_steering();       // nudge to changed card
    fn incident_2026_08_xx_shell_prompt_passes();  // wrong oracle
    fn incident_2026_08_xx_echo_satisfies_verify(); // self-reported evidence
    fn incident_2026_08_xx_tls_error_renders();    // interstitial passes
    fn incident_2026_08_xx_two_spend_writers();    // poller overwrites meter
    fn incident_2026_08_xx_profile_store_split();  // login vs agent profiles
    fn incident_2026_08_xx_archived_doing();       // nonsensical state combo
}
```

Each test reconstructs the incident's preconditions and asserts the architecture
rejects them structurally, not by a test that happens to check for them.

### Invariant 22: Deterministic orchestrator simulation

Instead of requiring real workers for all orchestration tests, the orchestrator runs
against a fake clock + fake provider + fake backend:

```
t=0   issue created
t=1   worker claims (lease 30s)
t=3   provider rate-limits
t=20  rate-limit resets
t=21  worker resumes
t=25  worker crashes (OOM)
t=26  lease reclaimed
t=27  worker-2 takes issue
t=40  worker-2 completes
t=41  verification passes
t=42  issue verified
```

Assert the entire event stream. Fuzz thousands of workflows in seconds. Catch race
conditions that Playwright will never reliably hit.

Use `proptest` for property/invariant testing:

```rust
// For arbitrary generated event sequences, assert:
proptest! {
    fn no_double_lease(events in arb_events()) {
        // an issue cannot have two live leases simultaneously
    }
    fn verified_implies_done(events in arb_events()) {
        // verified implies done occurred previously
    }
    fn blocked_dep_never_runnable(events in arb_events()) {
        // a blocked dependency can never be marked runnable
    }
    fn idempotent_replay(events in arb_events()) {
        // replaying the same events produces identical final state
    }
    fn no_unaudited_bypass(events in arb_events()) {
        // every force bypass has an audit entry with actor
    }
}
```

### Invariant 23: Server-side integration degradation

amux keeps orchestrating when external services disappear. Every integration has a
capability state:

```rust
enum IntegrationState {
    Available,
    Degraded { reason: String },
    Offline { since: DateTime<Utc> },
    AuthExpired,
    RateLimited { reset_at: DateTime<Utc> },
}

// The orchestrator checks before assignment:
// Issues requiring unavailable capabilities become capability-blocked,
// not repeatedly retried.
```

Internet disappears: local orchestrator keeps running. Claude disappears: Ollama
workers keep going. GitHub disappears: git operations queue. Gmail disappears: email
operations queue. The system degrades, names what's degraded, and recovers
automatically when connectivity returns.

### Invariant 24: Immutable event history

Every meaningful state mutation emits an append-only event:

```rust
struct DurableEvent {
    id: EventId,
    timestamp: DateTime<Utc>,
    actor: Actor,
    kind: EventKind,
    entity_id: String,  // issue, worker, schedule, etc.
    payload: Value,
}

enum EventKind {
    IssueCreated,
    IssueClaimed,
    IssueStarted,
    GateBlocked,
    GateSatisfied,
    WorkerMentioned,
    CommandQueued,
    CommandDelivered,
    TurnStarted,
    TurnCompleted,
    RateLimitEntered,
    RateLimitCleared,
    VerificationStarted,
    VerificationFailed,
    IssueVerified,
    ProviderDegraded,
    ProviderRecovered,
}
```

This is not event sourcing (current state is still the DB row). It is an append-only
audit log. It enables: debugging ("why did AR-421 end up here?"), replay (offline
sync), metrics (tokens per verified issue, time-in-state), and the `why-blocked`
query.

### Invariant 25: Priority and scheduling hints

Dependency graphs tell you what CAN run. Priority tells you what SHOULD run first.

```rust
struct SchedulingHints {
    priority: Priority,
    deadline: Option<DateTime<Utc>>,
    estimated_cost: Option<TokenCost>,
    preferred_worker: Option<WorkerId>,
    affinity: Vec<Affinity>,
}
```

The orchestrator scores candidates:

```
dependency critical path weight
+ explicit priority
+ age/starvation (prevent indefinite queue)
+ worker affinity (cached context reuse)
+ provider availability (don't assign to rate-limited)
+ estimated token cost (cheap work first when budget-constrained)
```

Without this, 380 TODO items becomes FIFO, which is the wrong order most of the time.

### Invariant 26: Backpressure on every channel

Rust removes Python's accidental serialization (GIL). Every async channel needs
explicit bounds and overflow semantics.

```rust
// Every mpsc channel has a bound:
let (tx, rx) = mpsc::channel::<DbWrite>(1024);        // DB write queue
let (tx, rx) = mpsc::channel::<WorkerEvent>(256);      // event channel
let (tx, rx) = mpsc::channel::<SseEvent>(64);           // per-subscriber SSE

// Overflow semantics are explicit per channel:
// DB writes: block sender (backpressure to API handler -> 503)
// WorkerEvents: drop oldest (stale events are worse than gaps)
// SSE: drop oldest + send "reconnect" hint
// Command queue per worker: bounded at 16, reject with 429
```

Never use an unbounded `mpsc`. Every queue's bound is a configuration value, not a
magic constant.

### Invariant 27: Immutable context snapshots

Every assignment records exactly what the worker received.

```rust
struct ContextSnapshot {
    id: ContextSnapshotId,
    issue_id: IssueId,
    worker_id: WorkerId,
    hash: Hash,
    fragments: Vec<FragmentRef>,
    total_tokens: u32,
    created_at: DateTime<Utc>,
}
```

Behavior becomes reproducible: "worker X failed AR-123 using context snapshot C-991."
Context caching becomes trivial: if the hash matches, reuse. Token optimization
becomes measurable: compare snapshot sizes across attempts.

### Invariant 28: Cheapest verifier first + evidence independence

Verification uses the cheapest/most deterministic verifier that can prove each
criterion:

| Criterion | Verifier | Cost |
|---|---|---|
| Tests green | `cargo test` exit code | Free |
| HTTP 200 | curl | Free |
| DOM contains element | Playwright assertion | Cheap |
| Artifact exists | `stat` | Free |
| Git commit merged | `git log --oneline` | Free |
| Screenshot visually correct | Model judgment or human | Expensive |
| Requirement semantically satisfied | Model judgment | Expensive |

Never call a model when a deterministic check suffices. This is Invariant 15 rule 1
applied to verification specifically.

**Evidence independence: verification cannot be satisfied solely by output produced
by the actor whose claim is being verified, when independent evidence is available.**

This principle comes directly from the commit history:

- `8bc9eb3`: a shell prompt was output, so the test said the provider was healthy
- `7870384`: the echoed prompt contained the expected words, so verification passed
- `9cd2892`: Chrome's "Privacy error" page had a title, so "dashboard renders" passed

The fix is structural: verifiers check externally observable outcomes, not
implementation activity.

```
BAD:  process emitted output
GOOD: provider returned structured result matching expected schema

BAD:  page title exists
GOOD: known amux DOM element is visible + hydrated + contains expected data

BAD:  worker said "tests passed"
GOOD: harness independently executed tests and observed exit code

BAD:  message was injected into terminal
GOOD: recipient acknowledged MessageId via WorkerEvent
```

```rust
enum VerifierKind {
    Command { cmd: String, expected_exit: i32 },
    HttpCheck { url: Url, expected_status: u16 },
    FileExists { path: PathBuf },
    PlaywrightAssertion { script: String },
    ModelJudgment { prompt: String },
    HumanReview,
}

struct Verification {
    kind: VerifierKind,
    evidence_source: EvidenceSource,
}

enum EvidenceSource {
    Independent,              // harness/external tool ran the check
    SelfReported,             // the actor being verified reported it
    Corroborated,             // self-reported + independently confirmed
}
```

Verifiers run in cost order. If the free checks fail, expensive ones never run.
When independent evidence is available, self-reported evidence alone is
insufficient -- the verifier must corroborate or independently confirm.

### Invariant 29: Message is a durable entity, not command plumbing

Steering messages, `@worker` mentions, issue discussion, and offline commands are all
the same thing: a **Message**. Making it an explicit durable entity gives you threads,
unread state, delivery tracking, search, and audit history without building each one
separately.

```rust
struct Message {
    id: MessageId,
    from: ActorRef,
    to: Vec<ActorRef>,       // worker/group/user/orchestrator
    issue_id: Option<IssueId>,
    thread_id: ThreadId,
    body: String,
    created_at: DateTime<Utc>,
    delivery: DeliveryState,
}

enum DeliveryState {
    Queued,
    Delivered { at: DateTime<Utc> },
    Read { at: DateTime<Utc> },
    Failed { reason: String },
}
```

`WorkerCommand::Steer` becomes `WorkerCommand::DeliverMessage(MessageId)`. The
orchestrator delivers messages at turn boundaries (Invariant 6). Offline messages
queue in IndexedDB and sync on reconnect (Invariant 14). `@worker-3` in an issue
description creates a Message addressed to worker-3 with the issue_id set. Threads
let a worker reply to a steering message and the reply appears in the issue activity.

### Invariant 30: Structured events for machines, append-only logs for humans

Two separate concepts that share correlation IDs but serve different consumers:

**Structured events** (`DurableEvent`, Invariant 24): machine-readable, typed,
queryable. The orchestrator, dashboard, and API consume these. They drive state
transitions, metrics, and the `why-blocked` query.

**Logs/output**: human-readable, append-only, unstructured text. Worker terminal
output, tool call results, error messages, debug traces. Humans read these when
debugging.

An issue detail exposes both, correlated by issue/worker/session/turn IDs:

```
Issue AR-421 detail:
  Activity          — messages + transitions (human timeline)
  Messages          — thread of steering/discussion
  Worker output     — raw terminal capture per turn
  Tool calls        — structured tool events
  Transitions       — board state machine history
  Gate evaluations  — gate checks with evidence
  Verification      — criteria + evidence + result
  System events     — orchestrator decisions, lease changes
```

Everything is cross-linked: clicking a gate evaluation shows the tool call that
produced the evidence, the turn it ran in, the worker output surrounding it, and the
message that triggered the work.

### Invariant 31: Compaction is a first-class subsystem

Context exhaustion is not an error. It is a lifecycle event with a defined protocol.
Compaction creates a cheaper derived context layer without destroying source history.

```rust
struct Compaction {
    id: CompactionId,
    worker_id: WorkerId,
    issue_id: Option<IssueId>,
    source_turns: Vec<TurnId>,
    summary: String,
    retained_facts: Vec<Fact>,
    retained_artifacts: Vec<ArtifactRef>,
    supersedes: Vec<ContextFragmentId>,
    token_before: u32,
    token_after: u32,
    created_at: DateTime<Utc>,
}
```

Lifecycle triggers:

```
context 70%  -> prepare compaction (build summary in background)
context 85%  -> compact (swap full history for compacted representation)
context 95%  -> checkpoint + new session
new session  -> hydrate: issue state + compacted history + unresolved work
```

**Source history is never replaced by compaction.** The original turns, messages,
logs, and artifacts remain in the DB. Compaction produces a `ContextFragment` with
`source: Compacted` that the context assembler (Invariant 16) uses instead of the
originals, but a worker or human can always drill into the full source.

Worker identity and issue assignment survive session replacement. The worker is
durable (Invariant 1); the session is ephemeral. A new session hydrates from the
compacted context and continues where the previous session left off.

### Invariant 32: Universal search without embeddings

A single search API spans every entity in the system. Basic search works completely
offline without spending tokens.

```
GET /api/search?q=rate+limited+anthropic+AR-421
```

Searchable entities:

```
issues, messages, workers, groups, turns, logs, tool calls,
verification evidence, gate evaluations, memories, files,
browser history/artifacts, email, calendar, CRM, schedules, events
```

Each result carries provenance:

```rust
struct SearchHit {
    entity_type: EntityType,
    entity_id: EntityId,
    scope: Scope,
    issue_id: Option<IssueId>,
    worker_id: Option<WorkerId>,
    timestamp: DateTime<Utc>,
    snippet: String,
    score: f32,
}
```

Search stack (no embeddings required for the first three tiers):

```
exact/filter lookup       — id, status, type, date range
        ↓
SQLite indexes            — indexed columns, foreign keys
        ↓
FTS5 lexical search       — full-text across all entities, offline
        ↓
optional semantic search  — embedding-based reranking (token cost, online only)
```

SQLite FTS5 is the baseline. `rate limited anthropic AR-421` works offline, instantly,
without spending tokens. Semantic search is an optional layer on top for fuzzy/concept
queries, with locally-generated and cached embeddings.

### The history/context layer

Invariants 29-32 are not four random features. They form one cohesive layer:

```
                  SEARCH (Inv 32)
                    ▲
                    │
 ┌────────┬─────────┼───────────┬──────────┐
 │        │         │           │          │
Issues  Messages   Events      Logs      Artifacts
(Inv 3) (Inv 29)  (Inv 30)   (Inv 30)
 │        │         │           │          │
 └────────┴─────────┼───────────┴──────────┘
                    │
               COMPACTION (Inv 31)
                    │
                    ▼
             Worker Context (Inv 16/27)
```

Two governing principles:

> **Everything produced by amux is durable, attributable, searchable, and selectively
> compactable. Original source data is never replaced by compaction.**

> **A user can navigate from any entity to any related entity without knowing where
> it was stored.** Clicking a gate evaluation reaches the tool call, the turn, the
> worker output, and the message that triggered the work.

### Invariant 42: Memory is a durable, scoped, revisioned entity

Memory is the largest architectural gap the commit history exposes. The recurring
pattern:

```
canonical state (worker memory)
     |
generated representation (MEMORY.md, compacted summary)
     |
gets read back as canonical
     |
old/generated data overwrites newer source
```

or:

```
worker A memory --+
                  +-> shared file -> last writer wins
worker B memory --+
```

The fix: memory is a first-class entity in SQLite, not a file.

```rust
struct MemoryEntry {
    id: MemoryId,
    scope: Scope,              // global, group, or worker
    name: String,              // kebab-case slug, unique within scope
    content: String,
    memory_type: MemoryType,   // user, feedback, project, reference
    version: u64,              // entity version (Invariant 35)
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,  // soft delete, never lose history
    provenance: Provenance,    // who created/updated this and why
}
```

The canonical store is the `memory_entries` table. Everything else is derived
(Invariant 39):

```
SQLite MemoryEntry (canonical)
      |
      +-> search index (FTS5, Invariant 32)
      +-> worker context (assembled by context pipeline, Invariant 27)
      +-> compacted summary (lossy, Invariant 31)
      +-> MEMORY.md projection (generated file, read-only)
      +-> inter-session API response (read from DB)

NONE of these write back to MemoryEntry without an explicit user mutation.
```

Scope isolation: a worker's memory is private to that worker. Group memory is
shared within the group. Global memory is visible to all. The scope resolver
(Invariant 2) determines which memories a worker sees at context assembly time.

Compaction may summarize old memories for prompt assembly (Invariant 31), but the
original `MemoryEntry` rows are never deleted or overwritten by compaction. The
compacted summary is a separate entity type (`CompactedContext`) that references
the source entries by ID.

Additive merging, deletion tracking, and concurrent writes are all handled by
the entity version + optimistic concurrency (Invariant 35). Two workers writing
to the same memory entry get a 409 conflict, not last-writer-wins.

### Design rule: self-documenting by construction

The system is its own documentation. No separate design doc, wiki, or README should
be required to understand what the system does, how it works, or why a decision was
made. This is enforced structurally, not by discipline:

1. **Types ARE the spec.** `WorkerCommand`, `WorkerEvent`, `BoardTransition`,
   `GateEvaluator`, `StallReason`, `ProviderState` -- reading the enum variants tells
   you exactly what the system can do. No prose description of "supported commands"
   that drifts from the code.

2. **API contract IS the documentation.** `JsonSchema` derives generate the OpenAPI
   spec from the same structs that handle requests. The spec cannot disagree with the
   implementation because it IS the implementation. `/api/spec.json` is always current.

3. **Error messages ARE the user guide.** Gate rejections return the exact gate
   criteria, the missing evidence, and the CLI command to satisfy them (Invariant 18).
   A 409 body teaches you what to do next. `why-blocked` returns the full chain.
   No separate "troubleshooting" doc.

4. **Event history IS the audit trail.** `DurableEvent` (Invariant 24) means every
   state transition is queryable: `amux issue AR-123 history` shows who did what,
   when, and why. No separate audit log to maintain.

5. **Test names ARE the requirements.** Each Playwright golden scenario and each
   proptest property IS a requirement. If the test passes, the requirement is met. If
   the test is missing, the requirement is unspecified.

6. **Config structure IS the admin guide.** Three-tier scope (Invariant 2) with
   `effective_config` means there is one way to configure anything, and
   `amux config show --effective --worker=X` shows exactly what is in effect and where
   each value came from (global, group, or worker override).

7. **The dependency graph IS the project plan.** `IssueRelation` (Invariant 4) means
   the board itself shows what blocks what. No separate Gantt chart or project tracker.

The bar: a new contributor should be able to understand the system by reading types,
running tests, and querying the API -- without opening a single markdown file.

---

## The Orchestrator (updated mental model)

```
USER / @MENTIONS / SCHEDULES
              |
              v
          BOARD GRAPH
    issues + gates + evidence
    + dependency resolution
    + priority scoring
              |
              v
         ORCHESTRATOR
 dependency / priority / quota
 + stall detection
 + lease management
 + provider routing
              |
              v
          ASSIGNMENT
      immutable context snapshot
      + token budget
              |
              v
           WORKER
              |
      WorkerCommand/Event
      (typed, durable, addressed)
              |
      +-------+--------+
      v                v
   PROVIDER        OPENCODE ◄── direct communication
 Claude/Gemini/  (prompts, messages,     BACKEND
 Codex/Ollama    cancel, events,       herdr/tmux/
      |          state queries)        native PTY
      |                |           (start/stop/inspect
      +-------+--------+            process only)
              |
              +--- process lifecycle --►
              v
          EVIDENCE
    (deterministic first,
     model judgment last)
              |
              v
        VERIFICATION
              |
              v
           VERIFIED
```

The implicit orchestrator (currently scattered across pickup, advance-nudge, steering,
snapshot, and session startup) becomes explicit:

```
                         ┌──────────────┐
                         │   Scheduler  │
                         └──────┬───────┘
                                │ fires
                                v
┌──────────────┐         ┌──────────────┐
│    Board     │<───────>│ Orchestrator │
│  + dep graph │         └──────┬───────┘
└──────┬───────┘                │ assignments
       │                        v
       │                 ┌──────────────┐
       │                 │   Workers    │
       │                 └──────┬───────┘
       │                        │ sessions
       │                        v
       │                 ┌──────────────┐
       │                 │   Sessions   │
       │                 └──────┬───────┘
       │                        │
       │              ┌─────────┼──────────┐
       │              │ OpenCode│ Backend  │
       │              │ (comms) │ (process)│
       │              │ prompts │ spawn    │
       │              │ messages│ stop     │
       │              │ cancel  │ inspect  │
       │              │ events  │ attach   │
       │              └─────────┴──────────┘
       v
┌──────────────┐
│ Verification │
└──────────────┘

  Scope/Context Resolution
         │
    ┌────┼────┐
    v    v    v
  Global Group Worker
    │    │    │
    └────┼────┘
         v
  ┌─────────────┐
  │   Context   │
  │  Assembler  │
  └──────┬──────┘
    ┌────┼────┐
    v    v    v
  Memory Env  Tools
```

The orchestrator's loop:

```rust
impl Orchestrator {
    async fn tick(&self, ctx: &AppContext) {
        // 1. Collect WorkerEvents from all active sessions
        let events = self.collect_events().await;

        // 2. Process events -> board transitions
        for event in events {
            match event {
                WorkerEvent::TurnCompleted(result) => {
                    self.evaluate_completion(result).await;
                }
                WorkerEvent::RateLimited(rl) => {
                    self.handle_rate_limit(rl).await;
                }
                WorkerEvent::Failed(f) => {
                    self.handle_failure(f).await;  // retry, escalate, or abandon
                }
                // ...
            }
        }

        // 3. Find runnable work (dependency graph + capabilities + scope)
        let runnable = self.board.runnable_issues().await;

        // 4. Match to available workers
        for issue in runnable {
            if let Some(worker) = self.find_capable_worker(&issue).await {
                self.assign(worker, issue).await;
            }
        }

        // 5. Check lease expirations
        self.reclaim_expired_leases().await;
    }
}
```

### Worker capability matching

Pickup is not "find next TODO card." It is:

```rust
struct WorkerCapabilities {
    tools: HashSet<Tool>,
    repositories: HashSet<Repo>,
    browser: bool,
    filesystem: FsScope,
    integrations: HashSet<Integration>,
    provider: Provider,
}

struct WorkRequirements {
    capabilities: HashSet<Capability>,
}

fn is_capable(worker: &Worker, issue: &Issue) -> bool {
    // runnable (deps met)
    // AND group-visible (scope isolation)
    // AND worker has required capabilities
    // AND worker is available (not at WIP limit, not rate-limited)
}
```

### Context assembly pipeline

When a worker picks up an issue, the orchestrator assembles its context:

```rust
trait ContextProvider: Send + Sync {
    async fn contribute(&self, req: &ContextRequest) -> Result<Vec<ContextFragment>>;
}

// Assembly order, with scope resolution at each layer:
// 1. Global instructions
// 2. Group instructions (override global)
// 3. Worker instructions (override group)
// 4. Issue context (description, deps, related issues)
// 5. Relevant memory (scoped: global, group, worker)
// 6. Environment / tool configuration (scoped)
// 7. Recent work / results from prior turns
// -> EffectiveContext (with token budget enforcement and provenance)
```

---

## Terminal backend evaluation

The terminal backend is the dominant complexity center in the Python system: ~90-100
tmux subprocess call sites, ~50 compiled regexes, 5 polling loops at
2s/3s/15s/60s/60s intervals, ~700 lines for rate-limit detection alone.

### What tmux costs today

1. **Character-level I/O**: `send-keys` injects text keystroke-by-keystroke. The 250-line
   `send_text` exists to fight autocomplete pickers, ghost text, Escape timing, and
   paste-buffer fallback.
2. **Scraping as control plane** (D1): 50+ regexes infer state from rendered terminal
   output. Breaks on any Claude Code UI string change.
3. **Polling overhead**: 5 loops, 40 subprocess calls per 60s cycle.
4. **Pane geometry fragility**: detached windows drift from 220x50.
5. **No structured lifecycle**: start is `new-session + send-keys`, stop is `send-keys /exit + hope`.

### Options evaluated

| Option | Description | Wins | Loses |
|---|---|---|---|
| **A: tmux improved** | tmux control-mode (`-CC`), persistent connection | Zero migration risk | 50 regexes stay, D1 stays |
| **B: herdr** | Process hosting, persistence, human terminal access | Session persistence, manual attach, recovery | No structured agent semantics on its own |
| **C: Native PTY** | Rust owns PTY via `portable-pty` | Zero subprocess overhead, streaming | Must solve persistence, lose manual attach |
| **D: OpenCode** | Structured agent protocol (commands, events, lifecycle) | IS the D1 exit, typed state | Depends on provider adoption |

### Recommendation

**Two layers: OpenCode for structured agent semantics + herdr for process hosting.**
tmux as fallback host. Native PTY as future option.

These are different concerns, not competing alternatives, and their boundaries
are strict:

- **OpenCode** provides the structured agent protocol -- typed commands, lifecycle
  events, state reporting. This is what eliminates terminal scraping as the control
  plane (the D1 exit). **All agent communication flows through OpenCode directly**:
  prompts, messages, cancellation, state queries, and events. The orchestrator
  talks to OpenCode, not to the backend, for anything involving agent semantics.

- **herdr** provides process hosting -- spawning, persisting, and providing human
  terminal access to agent sessions. It replaces tmux as the default process host
  because it is agent-oriented rather than terminal-oriented: cleaner lifecycle
  management, structured process state, no pane geometry drift. **herdr starts,
  stops, and lets humans inspect the OpenCode process. It does not carry prompts.**

The boundary: use herdr to start/stop/inspect the OpenCode process. Prompts,
messages, events, sessions, and cancellation go through OpenCode directly. This
eliminates most scraping entirely -- not by replacing `send-keys` with a
slightly better `send-keys`, but by removing the backend from the communication
path altogether.

Compared to the current tmux architecture:

- `send-keys` hacks are eliminated entirely (prompts go through OpenCode API)
- prompt delivery is typed and acknowledged (not fire-and-hope keystroke injection)
- state queries return structured data (not regex matches on rendered text)
- cancellation is a typed command (not Ctrl-C injection timing)

The structured agent semantics come from **OpenCode**, not from the process host:

- session/process lifecycle state (OpenCode reports, herdr hosts)
- waiting/blocked/completed detection (OpenCode structured events)
- turn boundaries and progress (OpenCode protocol)
- prompt/idle heuristics eliminated (OpenCode typed state)

The scraping goal is no longer "port all tmux scraping behavior to Rust." Instead:

1. OpenCode structured protocol is the sole communication channel
2. Provider hooks complement where available (Claude Code Stop/UserPromptSubmit)
3. Terminal output parsing is a fallback adapter for provider-specific signals
4. Scraping shrinks to liveness checks and rate-limit pattern detection only

tmux stays as a fallback process host behind the `SessionBackend` trait -- useful for
migration, debugging, and recovery. Native PTY (Option C) is a future target once
OpenCode + hooks cover enough that the scraper is liveness-only.

---

## Architecture

### Crate structure

```
amux/
  Cargo.toml                    # workspace root
  crates/
    amux-core/                   # shared types, scope resolution, no I/O
      src/
        lib.rs
        scope.rs                 # Scope enum, effective_config resolver
        board/
          mod.rs                 # Issue, BoardTransition, GateError
          graph.rs               # IssueRelation, dependency resolution
          state_machine.rs       # apply_transition (pure logic)
        worker/
          mod.rs                 # Worker, WorkerConfig, WorkerCapabilities
          command.rs             # WorkerCommand enum
          event.rs               # WorkerEvent enum
        session/
          mod.rs                 # Session, Turn, TurnId
        orchestrator/
          mod.rs                 # Orchestrator trait, WorkAssignment, Lease
          matching.rs            # capability matching
        verification.rs          # Verification, Criterion, Evidence
        provider.rs              # Provider enum (Claude/Gemini/Codex/Ollama)

    amux-server/                 # the binary -- HTTP, DB, runtime
      src/
        main.rs
        config.rs                # server.env, CLI args, three-tier config loading
        db/
          mod.rs                 # connection pool (single writer), WAL mode
          schema.rs              # migrations
          queries.rs             # typed query functions with GroupScope
        api/
          mod.rs                 # axum router
          workers.rs             # /api/workers/*, dead-letters, queue health
          board.rs               # /api/board/*
          scheduler.rs           # /api/schedules/*
          calendar.rs            # /api/cal-events/*, iCal, S3
          email.rs               # /api/email/*
          browser.rs             # /api/browser/*
          crm.rs                 # /api/crm/*
          files.rs               # /api/files/*
          journal.rs             # /api/journal/*
          graph.rs               # /api/graph/*
          proxy.rs               # /proxy/*
          settings.rs            # /api/prefs, /api/settings
          alerts.rs              # /api/alert/owner, push
          metrics.rs             # /api/metrics, /api/debug/*
          search.rs              # /api/search -- FTS5 across all entities
          messages.rs            # /api/messages/* -- durable message CRUD, threads
          sync.rs                # /api/sync -- delta sync, global rev (Invariant 35)
          auth.rs                # bearer token, share tokens, org
          sse.rs                 # /api/events -- revisioned StateEvents (Invariant 35)
          health.rs              # /health
          static_files.rs        # embedded dashboard
        orchestrator/
          mod.rs                 # runtime orchestrator loop
          reconcile.rs           # startup reconciliation
          pickup.rs              # runnable-issue selection
          context.rs             # context assembly pipeline
          compaction.rs          # context compaction lifecycle (70/85/95% triggers)
        runtime/
          mod.rs                 # job scheduling (DurableSchedule vs PeriodicTask)
          scheduler.rs           # user-facing durable schedules
          periodic.rs            # internal maintenance tasks
        backend/
          mod.rs                 # SessionBackend trait (process lifecycle ONLY)
          herdr.rs               # herdr process host (default): spawn, stop, inspect
          tmux.rs                # tmux process host (fallback): spawn, stop, inspect
          native_pty.rs          # native PTY (future/optional)
          adapter.rs             # terminal output -> WorkerEvent fallback translator
        opencode/
          mod.rs                 # AgentProtocol trait (direct agent communication)
          events.rs              # OpenCode -> WorkerEvent translation
          commands.rs            # WorkerCommand -> OpenCode translation
          transport.rs           # HTTP/WebSocket transport to OpenCode process
        provider/
          mod.rs                 # provider dispatch
          claude.rs              # Claude Code specifics (hooks, regexes, auth)
          gemini.rs              # Gemini specifics
          codex.rs               # Codex specifics
          ollama.rs              # Ollama specifics
        push/
          mod.rs                 # Web Push (VAPID, RFC 8291)
        ebook/
          mod.rs                 # EPUB/FB2/CBZ/MOBI reader
        torrent/
          mod.rs                 # aria2c RPC
        observability/
          mod.rs                 # tracing, correlation IDs
          trace.rs               # issue -> assignment -> worker -> session -> turn -> outcome

    amux-dashboard/              # build-time: embeds the SPA
      build.rs
      static/
        index.html
        app.js
        app.css
        sw.js
        manifest.json
        icons/

    amux-cli/                    # the `amux` command
      src/
        main.rs                  # clap subcommand tree
```

### SessionBackend trait (process lifecycle only)

The backend trait covers process hosting: start, stop, inspect. It does NOT
carry prompts, messages, or agent commands -- those go through OpenCode directly.
Higher layers never call herdr or tmux directly.

```rust
#[async_trait]
trait SessionBackend: Send + Sync {
    async fn spawn(&self, spec: SessionSpec) -> Result<ProcessRef>;
    async fn terminate(&self, session: &ProcessRef) -> Result<()>;
    async fn status(&self, session: &ProcessRef) -> Result<BackendStatus>;
    async fn attach_info(&self, session: &ProcessRef) -> Result<AttachInfo>;
    async fn reconcile(&self) -> Result<Vec<BackendSession>>;
}

enum BackendStatus {
    Running,
    Completed { exit_code: i32 },
    Crashed { signal: Option<i32> },
    NotFound,
}

struct AttachInfo {
    command: String,     // e.g. "herdr attach worker-name" or "tmux attach -t ..."
    pty_path: PathBuf,   // for human terminal access
}
```

`HerdrBackend` translates these to herdr's process hosting operations (spawn agent,
terminate, check liveness). `TmuxBackend` translates to `tmux new-session`,
`kill-session`, `has-session`, etc.

### OpenCode trait (agent communication)

All agent communication -- prompts, messages, cancellation, lifecycle queries --
goes through OpenCode directly, not through the backend.

```rust
#[async_trait]
trait AgentProtocol: Send + Sync {
    async fn send_prompt(&self, worker: &WorkerId, prompt: Prompt) -> Result<()>;
    async fn deliver_message(&self, worker: &WorkerId, msg: MessageId) -> Result<()>;
    async fn cancel(&self, worker: &WorkerId) -> Result<()>;
    async fn pause(&self, worker: &WorkerId) -> Result<()>;
    async fn resume(&self, worker: &WorkerId) -> Result<()>;
    async fn state(&self, worker: &WorkerId) -> Result<AgentState>;
    fn events(&self, worker: &WorkerId) -> impl Stream<Item = WorkerEvent>;
}

enum AgentState {
    Idle,
    Working { turn: TurnId, progress: Option<ProgressReport> },
    WaitingForInput,
    RateLimited(RateLimit),
    Paused,
    Exited(ExitStatus),
}
```

The orchestrator calls `AgentProtocol` for all worker interaction. It calls
`SessionBackend` only for process lifecycle (spawn on first assignment, terminate
on worker removal, reconcile on startup). This separation means:

- Switching backends (herdr -> tmux -> native PTY) changes nothing about how
  prompts, messages, or cancellation work
- OpenCode events stream regardless of which process host is running
- The backend never needs to understand prompt content or agent semantics
- `send-keys` hacks are eliminated entirely, not just reduced

### Key dependencies

| Concern | Crate | Notes |
|---|---|---|
| HTTP server | `axum` | async, tower middleware |
| Async runtime | `tokio` | multi-threaded, timers, process, signal |
| SQLite | `rusqlite` + `r2d2` | `bundled` feature, WAL mode, single-writer task |
| JSON | `serde` + `serde_json` | derive-based |
| SSE | `axum::response::sse` | built-in |
| TLS | `rustls` + `rcgen` | self-signed cert |
| Subprocess | `tokio::process` | herdr, tmux (fallback), git, node, browser-use |
| Embed files | `rust-embed` | dashboard baked into binary |
| CLI | `clap` | subcommand tree |
| Regex | `regex` | compiled pattern sets for terminal scraping |
| Tracing | `tracing` + `tracing-subscriber` | structured, correlation IDs |
| Web Push | `p256` + `hkdf` + `aes-gcm` | RFC 8291 |
| S3 | `aws-sdk-s3` | iCal feed |
| Gmail | `reqwest` | raw REST API |
| Cron | `cron` | schedule expression parsing |

### SQLite concurrency design

With 30+ jobs + HTTP + SSE + workers, SQLite needs explicit design:

- **WAL mode** always (concurrent readers, single writer)
- **Single writer task**: a dedicated `tokio::spawn` holds the write connection;
  mutations go through an `mpsc` channel. This prevents `SQLITE_BUSY` under Rust's
  real concurrency (Python's GIL accidentally serialized writes)
- **Busy timeout**: 5s for readers, writer never blocks (it IS the serialization point)
- **Connection pool**: `r2d2` for read-only connections (pool size = CPU cores)
- **Transaction boundaries**: one transaction per API request or orchestrator tick
- **Migration locking**: exclusive lock during schema migration, health endpoint
  returns 503 until complete
- **Backup**: periodic `.backup` to a second file; corruption recovery via PRAGMA
  integrity_check + restore from backup

---

## Migration strategy

### Phase 0: Foundation + golden scenario harness (est. 3 weeks)

**Goal**: binary that starts, serves the dashboard, manages the DB, AND the test
harness that will verify every subsequent phase.

1. Scaffold workspace, crate structure
2. `amux-core`: Scope, Worker, Session, Issue, BoardTransition, WorkerCommand/Event,
   Provider, StateRevision, EntityType, Mutation -- all types, no I/O. This is the
   system's vocabulary. Every entity type carries a `version: u64` field.
3. `amux-server/db`: all 51 tables as SQL migrations, WAL mode, single-writer task
4. `amux-server/config`: three-tier config loading (global/group/worker), `server.env`
5. `amux-server/api`: axum router, static file embedding, `/health`, auth,
   `/api/sync?since_rev=N` (Invariant 35), SSE with revisioned StateEvents
6. TLS setup with self-signed cert
7. **Golden scenario test harness** (Playwright-based): end-to-end scenario tests
   that will run against every phase. Start with:
   - Server starts, dashboard loads, health returns 200
   - Auth rejects bad token, accepts good token

**Test plan**:
- Unit: scope resolver merges global < group < worker correctly, worker wins conflicts
- Unit: scope resolver with group gates overriding global gates
- Unit: scope resolver with worker env overriding group env
- Unit: all 51 tables created in in-memory DB
- Unit: `BoardTransition` state machine rejects invalid transitions
- Unit: `BoardTransition` rejects nonsensical combos (archived + doing) (Invariant 3)
- Unit: Archive/Restore round-trip preserves all issue fields (Invariant 3)
- Unit: `IssueDisposition` is total -- every issue resolves to exactly one variant (Invariant 10)
- Unit: `WaitingFor` variants cover all non-terminal, non-runnable states (Invariant 10)
- Unit: `MutationResult.applied == false` when mutation is a no-op (Invariant 37)
- Unit: `#[serde(deny_unknown_fields)]` rejects unknown mutation fields (Invariant 37)
- Unit: `PagedResponse` always reports `total` >= `returned` (Invariant 40)
- Unit: API request/response types match OpenAPI spec (generated from JsonSchema derives)
- Unit: `DurableEvent` append succeeds for every `EventKind` variant (Invariant 24)
- Unit: backpressure -- bounded channels reject/drop correctly at capacity (Invariant 26)
- Unit: `ContextFragment` priority ordering is deterministic (Invariant 16)
- Unit: `GateEvaluator::Deterministic` runs before `Model` (Invariant 28)
- Simulation: fake clock + fake backend, orchestrator tick completes in <1ms (Invariant 22)
- Simulation: deterministic replay of 100 random event sequences produces identical state
- proptest: `BoardTransition` state machine rejects all invalid (from, to) pairs (Invariant 22)
- proptest: every non-terminal issue resolves to exactly one IssueDisposition (Invariant 10)
- proptest: no-op mutation never increments revision or entity version (Invariant 37)
- proptest: derived data never writes back to source (Invariant 39)
- proptest: scope merge is idempotent (merge(a, a) == a for arbitrary config)
- Integration: `GET /` returns dashboard HTML with version string
- Integration: `GET /health` returns 200 with build hash
- Integration: OpenAPI spec generated at `/api/spec.json`, valid per OpenAPI 3.1
- Integration: backend conformance suite passes for MockBackend (Invariant 21)
- Integration: provider conformance suite passes for MockProvider (Invariant 21)
- Playwright: dashboard loads in Chrome, no console errors
- Playwright: mobile viewport (375px) renders without overflow
- Playwright: offline mode -- cache shell, disconnect, dashboard still renders

### Phase 1: Workers + Orchestrator (est. 3 weeks)

**Goal**: create workers, start/stop them, orchestrator assigns work.

1. `amux-core/worker`: Worker struct, WorkerConfig, WorkerCapabilities
2. `amux-core/orchestrator`: Orchestrator trait, WorkAssignment, Lease
3. `amux-server/opencode/`: AgentProtocol impl -- direct communication with
   agents (prompts, messages, cancel, state queries, event stream). All agent
   interaction flows here, never through the backend.
4. `amux-server/backend/herdr.rs`: SessionBackend impl for herdr -- process
   lifecycle only (spawn, terminate, inspect, reconcile)
5. `amux-server/backend/tmux.rs`: SessionBackend impl for tmux -- process
   lifecycle only (fallback)
6. `amux-server/backend/adapter.rs`: terminal output -> WorkerEvent fallback
   translator (ANSI stripping, provider-specific rate-limit regexes). Used only
   for signals OpenCode/hooks do not expose structurally.
7. `amux-server/api/workers.rs`: CRUD, start (202 async), stop, peek, send
7. `amux-server/orchestrator`: runtime loop, startup reconciliation
8. SSE: worker state stream

The orchestrator runs from day one, even if its initial behavior is simple (pick up
next TODO, assign to idle worker). It grows in sophistication over phases.

**Test plan (per provider -- Claude, Gemini, Codex, Ollama)**:
- Unit: ANSI stripper handles test corpus
- Unit: Claude adapter translates 14 rate-limit patterns -> WorkerEvent::RateLimited
- Unit: Gemini adapter translates quota/daily-limit patterns -> WorkerEvent::RateLimited
- Unit: Codex adapter translates usage-limit pattern -> WorkerEvent::RateLimited
- Unit: Ollama adapter translates connection/model-not-found -> WorkerEvent::Failed
- Unit: WorkerEvent translation from sample terminal captures (corpus per provider)
- Unit: reconcile_on_startup handles all mismatch states (DB vs backend)
- Unit: lease expiration releases issue back to runnable
- Unit: stall_check fires when worker idle + non-terminal issue exists
- Unit: `ProviderQuota` state machine transitions for all `ProviderState` variants (Invariant 20)
- Unit: fallback chain routes to next-available provider when primary is exhausted (Invariant 20)
- Unit: execution state transitions are independent of board state transitions (Invariant 19)
- Unit: `@worker` mention parses from issue text, CLI, and dashboard input (Invariant 17)
- Unit: mention delivery state machine: Queued->Delivered->Acknowledged->ActedOn (Invariant 17)
- Unit: `Message` CRUD -- create, thread, delivery state tracking (Invariant 29)
- Unit: `Message` addressed to group fans out to all group members (Invariant 29)
- Unit: `WorkerCommand::Steer` wraps `DeliverMessage(MessageId)` (Invariant 29)
- Unit: command queue FIFO ordering within priority (Invariant 34)
- Unit: command queue rejects at capacity with 429 (Invariant 34)
- Unit: duplicate idempotency key returns existing result, no re-dispatch (Invariant 34)
- Unit: `CommandState` transitions: Queued->Dispatched->Delivered->Confirmed (Invariant 34)
- Unit: `DeliveryTiming::Immediate` bypasses turn boundary wait (Invariant 34)
- Unit: `DeliveryTiming::AtTurnBoundary` holds until turn ends (Invariant 34)
- Unit: `WorkerEvent` sequence numbers are monotonic per worker (Invariant 34)
- Unit: event gap detection flags missing sequence numbers (Invariant 34)
- Simulation: 50 workers, 200 issues, fake clock -- orchestrator assigns optimally with
  no double-leases (Invariant 22)
- Simulation: provider rate-limit + recovery -- fleet redistributes within 2 ticks (Invariant 20)
- Simulation: worker crash mid-issue -- lease reclaimed, issue re-assigned (Invariant 22)
- proptest: no double-lease for arbitrary event sequences (Invariant 22)
- proptest: verified implies done occurred previously (Invariant 22)
- proptest: no duplicate delivery for same idempotency key (Invariant 34)
- proptest: command queue FIFO preserved under arbitrary enqueue/dequeue (Invariant 34)
- proptest: dead-lettered command always has a DurableEvent (Invariant 34)
- Backend conformance: HerdrBackend passes process lifecycle suite (Invariant 21)
- Backend conformance: TmuxBackend passes process lifecycle suite (Invariant 21)
- Backend conformance: MockBackend passes process lifecycle suite (Invariant 21)
- Protocol conformance: OpenCodeProtocol passes communication suite (Invariant 21)
- Protocol conformance: MockProtocol passes communication suite (Invariant 21)
- Provider conformance: Claude adapter passes full suite (Invariant 21)
- Integration: create Claude worker on herdr, send prompt via OpenCode, receive events
- Integration: create Claude worker on tmux, send prompt via OpenCode, receive events
- Integration: create Ollama worker (`ollama run` backend), start, verify running
- Integration: switch worker from herdr to tmux, restart -- worker identity, issue
  ownership, messages, context all preserved (Invariant 33)
- Integration: SSE delivers worker state within 2s of WorkerEvent
- Integration: worker status transitions (idle->active->rate_limited->idle) reflected
  in API response within 1s
- Integration: `DurableEvent` emitted for every worker lifecycle transition (Invariant 24)
- Integration: `ContextSnapshot` recorded on every assignment (Invariant 27)
- Mock: SessionBackend mock for fast orchestrator unit tests
- Playwright: worker list renders, Start button responds within 1s (measured)
- Playwright: worker status badge updates within 2s of state change (all providers)
- Playwright: create worker with group assignment, verify group scope applied
- Playwright: idle worker with non-terminal issue -> dashboard shows stall warning
- Playwright: `@worker` mention in issue description triggers delivery (Invariant 17)
- Playwright: token budget dashboard shows tokens-per-verified-issue metric (Invariant 16)
- Playwright: message thread on issue detail -- send, reply, unread indicator (Invariant 29)

### Phase 2: Board + dependency graph (est. 3 weeks)

**Goal**: full board with gates, graph, scoped configuration, atomic claiming.

1. `amux-core/board`: Issue, IssueRelation, dependency graph, gate derivation
2. `amux-core/board/state_machine`: apply_transition with scope-aware gates
3. `amux-server/api/board.rs`: all routes, 409 gate contract, force+audit
4. Board auto-capture (prompt -> issue, derived title, no helper-model call)
5. Orchestrator integration: `board.runnable_issues()` uses dependency graph +
   capabilities + scope
6. Scoped gates: global gates, group overrides, worker specialization

**Test plan**:
- Unit: gate derivation for each (item_type, scope) combination
- Unit: global gate applies when group has no override
- Unit: group gate overrides global gate for same column transition
- Unit: worker-level gate overrides group gate
- Unit: gate inheritance chain: global defines 3 gates, group removes 1, worker adds 1
  -> effective gates are correct
- Unit: column definitions scoped to group (group A has 5 columns, group B has 3)
- Unit: dependency graph: A blocks C, B blocks C, both complete -> C runnable
- Unit: dependency graph: circular dependency detected and rejected at create time
- Unit: atomic claim: two concurrent claims, exactly one succeeds (sqlx test with
  two connections)
- Unit: lease expires -> issue reclaimable, original worker's claim is void
- Unit: `force=true` bypasses gate, writes audit trail including actor + reason
- Unit: `Gate` entity CRUD -- create, scope, version, history (Invariant 18)
- Unit: `why-blocked` query returns gate id, criterion, missing evidence, suggested
  command (Invariant 18)
- Unit: `GateEvaluator` ordering: `Deterministic` before `Model` (Invariant 28)
- Unit: issue state vs execution state separation -- rate-limit changes execution
  state only, never board state (Invariant 19)
- Unit: priority scoring: critical-path weight + explicit priority + age starvation +
  affinity + provider availability + cost (Invariant 25)
- proptest: dependency graph is acyclic for arbitrary relation insertions (rejects cycles)
- proptest: force bypass always produces audit entry with actor (Invariant 22)
- proptest: `IssueRelation::Blocks` and `IssueRelation::DependsOn` are inverse-consistent
- Simulation: 100 issues with complex dependency graph, orchestrator resolves runnable
  set in topological order (Invariant 22)
- Integration: create parent + children, complete children, parent becomes runnable
- Integration: board CRUD through full lifecycle (todo->claimed->doing->review->done
  ->verified) with proper gate acks at each transition
- Integration: group A board has custom columns, group B has default columns, both
  work independently
- Integration: API responses match OpenAPI contract for every board endpoint
- Integration: `DurableEvent` emitted for every board transition (Invariant 24)
- Integration: `why-blocked` API returns actionable gate info (Invariant 18)
- Playwright: board renders, drag-and-drop transitions work, gate 409 shown as toast
  with the exact gate criteria and CLI command to satisfy
- Playwright: mobile board usable at 375px, touch targets >= 44px
- Playwright: user creates issue in group A, worker in group B cannot see it
- Playwright: no-stall check -- complete an issue, verify worker picks up next or
  goes idle with all issues terminal
- Playwright: `why-blocked` detail panel shows criteria, evidence, suggested CLI (Invariant 18)

### Phase 3: Scheduling (est. 2 weeks)

**Goal**: user-facing durable schedules + internal periodic tasks, kept separate.

User schedules (durable):
- Persist in DB with run history, missed-run behavior, timezone semantics, retry policy
- Scoped to global/group/worker
- Audit trail (X-Amux-Session attribution on every mutation)

Internal periodic tasks (ephemeral):
- In-memory, no run history needed
- `tokio::time::interval`, not threads with `time.sleep`
- A slow task doesn't block others (spawned as separate tokio tasks)

```rust
// These are different things with different semantics
struct DurableSchedule { /* DB-backed, history, retry */ }
struct PeriodicTask { /* in-memory, interval, fire-and-forget */ }
```

**Test plan**:
- Unit: cron expression parser handles all formats (daily, every Nm, weekday, 5-field)
- Unit: schedule CRUD respects audit trail
- Unit: missed-run behavior (skip vs. catch-up)
- Unit: `DurableSchedule` vs `PeriodicTask` are separate types with separate lifecycles
- Unit: schedule scoped to group only fires for workers in that group (Invariant 2)
- Integration: create schedule, run-now, verify `schedule_runs` with `source` field
- Integration: periodic task ticks at interval, does not block other tasks
- Integration: `DurableEvent` emitted for schedule fire, manual run, missed run (Invariant 24)
- Playwright: schedule list, create, edit, run-now button works

### Phase 4: Control plane (steering, rate-limit, auto-responder) (est. 2 weeks)

**Goal**: WorkerCommand delivery, WorkerEvent processing, message delivery, and
compaction subsystem.

1. Command queue: DB-backed per-worker queue with delivery protocol (Invariant 34)
2. WorkerCommand dispatch through AgentProtocol (OpenCode) with delivery confirmation
3. OpenCode -> WorkerEvent translation (structured lifecycle), terminal adapter as
   fallback for provider-specific rate-limit detection
4. Scan demotion: hook-reported workers get demoted capture frequency
5. Auto-responder for `--dangerously-skip-permissions` workers
6. Turn tracking: TurnStarted/TurnCompleted events drive the orchestrator
7. Message delivery: `Message` entities delivered at turn boundaries (Invariant 29)
8. Dead-letter handling: commands that exhaust retries produce StallViolation (Invariant 34)
9. Compaction subsystem: context 70% -> prepare, 85% -> compact, 95% -> checkpoint +
   new session, new session -> hydrate from compacted context (Invariant 31)

**Test plan**:
- Unit: steering dedup prevents double delivery
- Unit: rate-limit regexes match all known formats per provider (14 patterns for Claude,
  2 for Gemini, 1 for Codex, 1 for Ollama)
- Unit: scan demotion correctly classifies hook-reported vs. hookless
- Unit: WorkerEvent translation from all known terminal states
- Unit: backpressure -- command queue per worker bounded at 16, rejects with 429 (Invariant 26)
- Unit: backpressure -- SSE channel drops oldest + sends reconnect hint on overflow (Invariant 26)
- Unit: `ContextSnapshot` created on every assignment, hash stable for identical content (Invariant 27)
- Unit: context assembly priority: issue > deps > memory > turns > history (Invariant 16)
- Simulation: 10 workers rate-limiting simultaneously, orchestrator redistributes
  to available providers within 3 ticks (Invariant 20/22)
- Simulation: command delivery under backpressure -- no lost commands, 429 for overflow (Invariant 26)
- Simulation: server restart with 5 pending commands -- all redelivered via
  idempotency, no duplicates (Invariant 34)
- Simulation: command with precondition -- entity changes before delivery,
  command expires with PreconditionResult::Failed (Invariant 38)
- Simulation: human message has no precondition, always delivers (Invariant 38)
- Simulation: command dispatch fails 2x then succeeds -- retry backoff, final
  Confirmed state (Invariant 34)
- Simulation: command exhausts 3 retries -- dead-lettered, StallViolation emitted,
  dashboard alert (Invariant 34)
- Simulation: 40 workers, mixed DeliveryTiming -- Immediate commands bypass turn
  boundary, AtTurnBoundary commands wait, WhenIdle commands queue (Invariant 34)
- Integration: enqueue command, verify delivery within 4s
- Integration: command survives server restart -- pending command redelivered (Invariant 34)
- Integration: dead-letter visible via `GET /api/workers/:id/dead-letters` (Invariant 34)
- Integration: queue depth reflected in worker health, deep queue warns (Invariant 34)
- Integration: rate-limit auto-wait fires on simulated terminal output
- Integration: `IntegrationState` transitions reflected in `/health` endpoint (Invariant 23)
- Integration: Gmail unavailable -> email operations queue, recover on reconnect (Invariant 23)
- Integration: message delivered at turn boundary, not mid-turn (Invariant 29)
- Integration: offline message queued, delivered on reconnect (Invariant 29)
- Integration: compaction at 85% context -- compacted fragment created, source turns
  preserved, token_after < token_before (Invariant 31)
- Integration: MemoryEntry CRUD -- scope isolation, version increments, soft delete (Invariant 42)
- Integration: MEMORY.md generated from MemoryEntry table, read-only (Invariant 39/42)
- Integration: compacted summary references source entries by ID, never overwrites (Invariant 39)
- Integration: concurrent memory writes to same entry -> 409 conflict (Invariant 42)
- Incident regression: incident_2026_07_30_duplicate_draft (Invariant 41)
- Incident regression: incident_2026_07_30_board_read_after_write (Invariant 41)
- Incident regression: incident_2026_08_xx_stale_steering (Invariant 38/41)
- Integration: context 95% triggers checkpoint + new session, new session hydrates
  from compacted context (Invariant 31)
- Integration: compaction never deletes source turns/messages/logs (Invariant 31)
- Simulation: context exhaustion cycle: 10 turns -> compact -> 10 more turns -> new
  session -> hydrate -> continue (Invariant 31)
- Playwright: worker status updates live in dashboard, rate-limit shown within 2s
- Playwright: provider quota dashboard shows fleet-level capacity (Invariant 20)
- Playwright: compaction indicator on worker card when context > 70% (Invariant 31)
- Playwright: dead-letter badge on worker card when commands fail delivery (Invariant 34)
- Playwright: queue health warning when delivery rate < 90% (Invariant 34)

### Phase 5: Verification (est. 2 weeks)

**Goal**: verification as a first-class lifecycle stage, not a manual flag.

1. `Verification` struct with criteria, evidence, result
2. Verification pipeline: done -> verification execution -> verified or rejected
3. Playwright-based acceptance tests for every user-facing flow
4. Integration with CI: `verified` requires green pipeline + prod confirmation

The user flow acceptance tests built here become the regression suite:

- User submits work to a worker via the dashboard
- Work gets decomposed into issues on the board
- Issues get picked up by workers (orchestrator assigns)
- Issues flow through board statuses with gate acknowledgments
- Completed issues go through verification
- Failed verification returns to in-progress
- Successful verification closes the issue

**Test plan**:
- Unit: verification state machine (done->verification->verified|rejected->in_progress)
- Unit: cheapest-verifier-first ordering: Command < HttpCheck < FileExists <
  PlaywrightAssertion < ModelJudgment < HumanReview (Invariant 28)
- Unit: free verifier failure short-circuits -- model verifier never called (Invariant 28)
- Simulation: verification pipeline with mixed verifier types, cost-ordered execution
- Integration: issue completes, verification runs, evidence recorded
- Integration: verification fails, issue returns to doing with rejection reason
- Integration: `DurableEvent::VerificationStarted` and `VerificationFailed`/`IssueVerified`
  emitted with full evidence chain (Invariant 24)
- Integration: issue detail API returns all correlated views: activity, messages, worker
  output, tool calls, transitions, gate evaluations, verification evidence (Invariant 30)
- Integration: clicking a gate evaluation traces to the tool call, turn, and worker
  output that produced the evidence (Invariant 30)

**Playwright golden scenarios (the acceptance criteria)**:

Each scenario runs end-to-end in a real browser using herdr as the default backend.
Timing is measured and asserted. At least one complete scenario (the happy path)
executes identically with `AMUX_BACKEND=herdr` and `AMUX_BACKEND=tmux`, producing
the same board transitions, WorkerEvents, verification result, and final issue state
(Invariant 33).

1. **Happy path (per provider: Claude, Gemini, Codex, Ollama)**:
   - User opens dashboard, submits work text to a worker via the UI
   - Work gets decomposed into issues on the board (< 5s)
   - Orchestrator assigns issue to worker (< 3s)
   - Worker picks up, starts executing (status shows "active" within 2s)
   - Worker completes, issue moves to "done" (status shows "idle" within 2s)
   - Verification runs (Playwright checks the work, evidence recorded)
   - Issue moves to "verified" -- terminal state
   - **Assert**: no stalls at any point. Worker status was never stale > 2s.

2. **Failure + retry**:
   - Worker fails an issue (verification rejects)
   - Issue returns to "doing" with rejection reason visible in UI
   - Worker retries, succeeds
   - Issue reaches "verified"
   - **Assert**: rejection reason displayed as toast and in issue detail

3. **Rate limit recovery (per provider)**:
   - Worker hits rate limit during work
   - Dashboard shows "rate limited" status within 2s (not stale "active")
   - Reset time displayed in UI
   - Worker auto-resumes after reset
   - Issue continues to completion
   - **Assert**: no manual intervention required

4. **Dependency chain**:
   - Create parent with 3 children
   - Children assigned to workers, run concurrently
   - As each child completes, parent remains blocked
   - All children done -> parent becomes runnable -> assigned -> completed -> verified
   - **Assert**: dependency graph respected, no premature pickup

5. **Scoped gates**:
   - Group A requires code review gate, Group B does not
   - Worker in Group A completes issue -> blocked at review gate
   - Worker in Group B completes issue -> moves straight to done
   - **Assert**: gate enforcement matches group scope

6. **Offline mode**:
   - Dashboard goes offline (network disconnect)
   - User creates 3 board cards, sends 2 worker commands
   - Dashboard reconnects
   - All 5 queued operations replay successfully
   - **Assert**: all operations applied, no duplicates, conflicts shown as toasts

7. **No-stall invariant**:
   - Create 5 issues, start 2 workers
   - Workers process issues
   - At every 5s checkpoint: no worker is idle with non-terminal issues in its scope
   - All issues reach terminal state
   - **Assert**: zero stall violations across entire run

8. **Multi-provider fleet**:
   - Start 1 Claude worker, 1 Gemini worker, 1 Ollama worker
   - Assign different issues to each
   - All three complete independently
   - **Assert**: each provider's status updates are timely, no cross-provider confusion

9. **Backend interchangeability (Invariant 33)**:
   - Run the happy path with `AMUX_BACKEND=herdr` (default)
   - Run the identical happy path with `AMUX_BACKEND=tmux`
   - **Assert**: same board transitions, same WorkerEvents, same verification result,
     same final issue state. The backend is invisible above the `SessionBackend` trait.

10. **Real-time convergence (Invariant 35)**:
    - Open two browser tabs
    - Tab 1: create 10 board cards rapidly
    - Intercept SSE: drop every 3rd event to Tab 2
    - Tab 2 detects rev gap, delta syncs
    - Both tabs show identical state
    - Kill server, restart
    - Both tabs reconnect, delta sync from their last rev
    - Mutate same issue from both tabs simultaneously
    - Loser gets 409, reconciles
    - **Assert**: both tabs converge to identical, revision-consistent state.
      No stale, duplicate, or missing entities at any point after convergence.

### Phase 6: Email, Calendar, CRM (est. 2 weeks)

**Goal**: integration subsystems, each scoped.

1. Email: Gmail OAuth2 via `reqwest`, send/reply/inbox/search. Scoped to worker.
2. Calendar: events CRUD, iCal generation (RFC 5545), S3 upload. Scoped to global.
3. CRM: contacts, tags, interactions. Scoped to global.

**Test plan**:
- Unit: iCal RFC 5545 (line folding, UTC, VALUE=DATE)
- Unit: CRM CRUD on in-memory DB
- Integration: Gmail OAuth token refresh (mock HTTP)
- Integration: S3 upload (LocalStack or mock)
- Playwright: email compose, calendar event creation, CRM contact card

### Phase 7: Browser profiles, files, misc (est. 2 weeks)

**Goal**: remaining subsystems.

1. Browser profiles: native Chrome profile management (no Python browser-use dep),
   CDP-direct screenshot/navigation, profile inventory with saved-auth tracking,
   lock-file cleanup on startup, and a clean split between profile management (always
   free) and AI-driven browsing (model call only when needed)
2. Files: browse, upload, download, ebook reader
3. Push notifications: VAPID + RFC 8291
4. Graph, journal, proxy, torrent, alerts, metrics

**Test plan**:
- Unit: Web Push encryption roundtrip
- Unit: VAPID JWT generation
- Integration: browser profile create -> start -> screenshot -> stop lifecycle
- Integration: profile lock-file cleanup on server restart
- Integration: CDP screenshot matches expected dimensions
- Integration: file upload/download roundtrip
- Integration: push subscription lifecycle
- Playwright: browser tab shows profile inventory with auth domains
- Playwright: start profile, navigate, screenshot renders in dashboard
- Playwright: file browser navigable

### Phase 8: Dashboard + CLI (est. 2 weeks)

**Goal**: extract SPA, build CLI binary.

1. Extract 44k-line inline SPA into `amux-dashboard/static/`
2. `rust-embed` for compile-time inclusion
3. Version stamping via `build.rs`
4. CLI: `clap` subcommand tree mirroring the bash script
5. Rename all `session` references to `worker` in dashboard + CLI

**Test plan**:
- Integration: served dashboard matches extracted source
- Integration: service worker caches shell URLs
- Integration: `amux board add "test"` creates card, prints ID
- Integration: `amux send <worker> "hello"` delivers
- Playwright: all dashboard tabs render, PWA offline works
- Playwright: SSE delivers revisioned StateEvents, client applies in rev order (Invariant 35)
- Playwright: rev gap triggers delta sync (drop SSE events, verify convergence) (Invariant 35)
- Playwright: two tabs mutate same issue -> both converge (Invariant 35)
- Playwright: kill server, restart -> client reconnects and delta-syncs (Invariant 35)
- Playwright: 1,000 rapid board mutations -> UI finishes at exact backend rev (Invariant 35)
- Playwright: connection indicator shows LIVE/STALE/OFFLINE/SYNCING (Invariant 35)
- Playwright: optimistic write rejected (409) -> rollback visible to user (Invariant 35)

### Phase 9: Observability + performance (est. 2 weeks)

**Goal**: correlation-ID tracing and performance baselines.

Every operation is traceable: issue -> assignment -> worker -> session -> turn ->
command -> tool -> outcome. Correlation IDs flow through the entire stack.

```
Issue #421
└─ assigned worker-3
   └─ session s-8821
      └─ turn t-4
         └─ blocked waiting on #419
            └─ #419 verification failed: test X
```

Generalized `why` query -- not just "why is this stuck" but "why did this happen":

```
amux why issue AR-42       # full provenance chain
amux why worker backend    # current state + how it got there
amux why command CMD-83    # dispatch path, precondition result, delivery
amux why schedule SCHED-108 # last N fires, source (cron/manual), outcomes
amux why integration gmail  # auth state, last success/failure, degradation
```

All answered from structured provenance (Invariant 24), not grep over logs.

Performance measurement:
- Dashboard load time (target: <500ms cold, <100ms cached)
- SSE event latency (target: <2s from state change to client)
- Worker start time (target: <3s from button press to running)
- API response time p50/p95/p99 for all routes
- Memory usage (target: <200MB RSS with 40 workers idle)
- CPU usage (target: <5% at idle with 40 workers)

**Test plan**:
- Integration: correlation IDs present in all log entries for a traced operation
- Integration: dashboard "why is this stuck?" query returns full trace
- Integration: `GET /api/search?q=...` returns hits across issues, messages, events,
  logs, workers, schedules, email, CRM (Invariant 32)
- Integration: search result provenance -- every `SearchHit` carries entity_type,
  scope, issue_id, worker_id, timestamp (Invariant 32)
- Integration: FTS5 search works completely offline (Invariant 32)
- Integration: exact/filter lookup -> SQLite index -> FTS5 -> optional semantic
  reranking stack (Invariant 32)
- Integration: structured events vs logs -- same issue, both views present, correlated
  by turn_id (Invariant 30)
- Playwright: universal search bar -- type query, results span all entity types with
  provenance chips (Invariant 32)
- Playwright: search result click navigates to entity detail with context (Invariant 32)
- Performance: FTS5 search over 10k entities returns < 50ms (Invariant 32)
- Performance: all latency targets met under load (40 workers, 100 board items)
- Performance: RSS stays flat over 24h soak test
- Performance: no file descriptor leaks over 24h

### Phase 10: CI/CD pipeline (est. 1 week)

**Goal**: zero-regression guarantee.

Pipeline stages:
1. `cargo check` + `clippy` -- compile-time correctness
2. `cargo test` -- all unit + integration + simulation + proptest tests
3. Backend conformance suite -- MockBackend + HerdrBackend + TmuxBackend (Invariant 21)
4. Provider conformance suite -- MockProvider + all adapter tests (Invariant 21)
5. Playwright suite -- all golden scenarios + all UI flows
6. Performance benchmarks -- compared against baseline, regression = failure
7. SQLite migration test -- apply migrations to a copy of prod DB, verify no data loss
8. Binary size check -- regression if binary grows >20%

Regression detection:
- Latency regression: any p95 increase >10% vs baseline is a CI failure
- Memory regression: RSS increase >20% vs baseline is a CI failure
- Feature regression: any Playwright scenario that was green and turns red blocks merge

The pipeline runs on every PR. No merge without green.

### Phase 11: Migration + go-live (est. 2 weeks)

**Goal**: zero-downtime cutover from Python to Rust with full data migration.

#### Data migration

SQLite schema is preserved, so the DB file is directly compatible. But:

1. **Schema diff**: run both servers' migration code against the same DB, diff the
   resulting schemas. Any mismatch blocks go-live.
2. **Data validation**: for every table, verify row counts match and spot-check
   content (especially `issues`, `schedules`, `prefs`, `email_events`).
3. **Worker config migration**: `.env` files -> validated `WorkerConfig` structs.
   Any validation failure produces a report, not a silent skip.

#### Cutover sequence

1. **Week 1: shadow mode.** Rust server runs on port 8823 alongside Python on 8822.
   Both read the same DB (WAL allows concurrent readers). Dashboard traffic stays on
   Python. Automated traffic replay compares responses.
2. **Week 2: swap.** Update launchd plist to start Rust binary on 8822. Python moves
   to 8823 as fallback. Monitor for 48h.
3. **Day 15: cut fallback.** If no rollback needed, stop Python server. Keep binary
   available for 30 days.

#### Backend migration

The Rust rebuild migrates toward herdr as the primary backend:

- herdr is the default for new workers (`BackendKind::Herdr`)
- Existing tmux-based workers continue on tmux during migration
- Workers can be individually switched between herdr and tmux via config
  (`backend: BackendKind::Tmux` override)
- Backend choice does not require DB/schema changes
- Backend choice does not change worker identity or issue ownership
- A worker can restart on a different backend while preserving all durable
  amux state (Invariant 33)

tmux remains available as rollback/fallback throughout, not as a parallel
control plane. There is one orchestrator, one WorkerCommand protocol, one
WorkerEvent protocol, one worker state machine, and one reconciliation system.
herdr and tmux only translate those primitives to/from their underlying
process-host mechanisms.

#### Rollback plan

At any point during shadow or swap:
1. Stop Rust server
2. Start Python server on 8822
3. DB is compatible in both directions (no destructive migrations)

Backend rollback: switch any worker from herdr back to tmux by changing its
`backend` config and restarting. No data migration, no identity change.

#### Cloud deployment

1. Update `deploy-cloud.yml` to build Rust Docker image
2. Rust binary built with `--target x86_64-unknown-linux-musl` for Alpine containers
3. Same single-codebase rule: one binary, no cloud/local branching

## Estimated timeline

| Phase | Duration | Running total |
|---|---|---|
| 0 - Foundation + test harness | 3 weeks | 3 weeks |
| 1 - Workers + Orchestrator | 3 weeks | 6 weeks |
| 2 - Board + dependency graph | 3 weeks | 9 weeks |
| 3 - Scheduling | 2 weeks | 11 weeks |
| 4 - Control plane | 2 weeks | 13 weeks |
| 5 - Verification | 2 weeks | 15 weeks |
| 6 - Email/Calendar/CRM | 2 weeks | 17 weeks |
| 7 - Browser/files/misc | 2 weeks | 19 weeks |
| 8 - Dashboard + CLI | 2 weeks | 21 weeks |
| 9 - Observability + performance | 2 weeks | 23 weeks |
| 10 - CI/CD pipeline | 1 week | 24 weeks |
| 11 - Migration + go-live | 2 weeks | **26 weeks** |

~6 months. Phases 0-5 (the core: orchestrator, board, workers, verification) take 15
weeks and produce a functionally complete system. Phases 6-7 are the integration long
tail (parallelizable, saves ~2 weeks). Phases 8-11 are polish, testing, and cutover.

## Risks

1. **Dashboard compatibility**: 44k lines of JS talking to 212 API routes. Any response
   shape mismatch breaks the UI. Mitigation: `serde` structs match Python's exact
   response shapes; Playwright catches regressions.
2. **Feature velocity**: amux gains ~2-3 features/week in Python. During the rewrite,
   development must continue. Mitigation: Python stays the dev target until phase 11
   swap; the golden scenario harness catches drift.
3. **Terminal scraping residue**: provider-specific rate-limit regexes must still be
   ported for signals OpenCode/hooks cannot expose structurally. Mitigation: OpenCode's
   structured agent protocol handles most lifecycle transitions directly; scraping
   scope is reduced to provider-specific rate-limit patterns only. Extract test corpus
   from Python, run as unit tests per provider.
4. **SQLite under real concurrency**: Python's GIL accidentally serialized writes. Rust
   exposes latent races. Mitigation: single-writer task, WAL mode, explicit transaction
   boundaries designed in phase 0.
5. **Scope resolution complexity**: three-tier inheritance with overrides is easy to
   spec, hard to get right in every query. Mitigation: one resolver function in
   `amux-core`, used by all consumers. Never re-derive scope logic per-query.

## Lessons from the Python system (fix these structurally, not by porting)

These are real incidents from the last 72 hours of operating amux at scale. Each one
points to an architectural flaw that the Rust rebuild must not inherit.

### L1: The 6MB board payload

The board API returns every card including full `desc` fields. One card has a 94KB
desc. The default response is 6.2MB, of which 4.4MB (74%) is desc text the dashboard
never renders (it shows `desc.split('\n')[0].slice(0, 80)`). Every SSE push, every
poll, every page load ships this. On a phone over cellular, this is the dominant
latency source.

**Rust fix**: the API has two shapes by design.
- **List responses** (`GET /api/board`, SSE pushes): `desc` truncated to first line,
  `desc_truncated: true` flag set. Full desc is never in a list payload.
- **Detail responses** (`GET /api/board/:id`): full desc, full log, full history.

This is not an optimization to add later. It is a response type definition:

```rust
#[derive(Serialize)]
struct BoardItemSummary {
    // all fields EXCEPT desc/log are full
    desc_preview: String,  // first line, max 200 chars
    desc_truncated: bool,
    // no `log` field at all
}

#[derive(Serialize)]
struct BoardItemDetail {
    // everything, full desc, full log
    desc: String,
    log: String,
}
```

Delta sync (Invariant 14) compounds this: after the initial load, the client receives
only changed items, not the full board. A single card status change pushes ~200 bytes
instead of 6.2MB.

### L2: Process-host mechanics must not leak above the backend adapter

The original tmux outage: the `=` prefix for exact session matching works for
session-level commands (`has-session`, `kill-session`) but silently fails for
pane-level commands (`capture-pane`, `send-keys`). Every capture and send-keys
across 62 sessions was silently failing, and the test suite only verified the
commands that happened to work.

The lesson is broader than "encapsulate tmux targeting." **Process-host-specific
mechanics must be confined to a replaceable backend adapter. amux must not depend on
terminal multiplexer behavior for correctness.** Each backend has its own addressing
semantics -- tmux pane targets vs. herdr agent references vs. PTY file descriptors
-- and all of them are implementation details behind `SessionBackend`.

**Rust fix**: the `SessionBackend` trait encapsulates all backend-specific interaction.
No raw `subprocess::Command` construction outside the backend module. The backend
conformance suite (Invariant 21) exercises every operation against every backend
implementation -- not just the ones that motivated the original fix.

```rust
impl HerdrBackend {
    fn agent_ref(&self, worker: &str) -> String {
        format!("amux-{}", worker)  // herdr agent reference
    }
    // Every herdr operation goes through this -- no raw agent name construction elsewhere
}

impl TmuxBackend {
    fn target(&self, worker: &str) -> String {
        format!("=amux-{}:", worker)  // exact + pane resolution
    }
    // Every tmux operation goes through this -- no raw "-t" construction elsewhere
}
```

### L3: Board items not flowing

380 todo items, 25 doing with no session, steering messages piling up undelivered.
The orchestrator logic is scattered across pickup, advance-nudge, steering, and
snapshot -- and when any one piece breaks (as capture-pane did), the others don't
compensate. There is no single place that answers "why isn't this issue moving?"

**Rust fix**: the explicit Orchestrator (Invariant 10) runs a stall check on every
tick. When it detects a stall, it produces a `StallViolation` with the reason:

```rust
// StallReason is the PROBLEM; WaitingFor (Invariant 10) is the STRUCTURED STATE.
// A StallViolation fires when an issue has no WaitingFor and no assigned worker.
enum StallReason {
    WorkerIdle,                          // worker has capacity but isn't assigned
    NoCapableWorker,                     // no worker can do this work
    ProcessDown { error: String },       // backend reports process not running
    ProtocolUnreachable { error: String }, // OpenCode not responding
    Orphaned,                            // assigned to a worker that no longer exists
    CommandExpired { command: CommandId }, // precondition failed at delivery (Inv 38)
}
// Note: rate-limited, dependency-blocked, and gate-blocked are WaitingFor variants,
// not stalls. They have structured wait reasons and expected resolution paths.
```

The dashboard shows stall reasons inline on each card. A user looking at the board
can see exactly WHY each item is stuck, not just that it is.

### L4: No progress heartbeat

"I have no means of knowing if progress is continuing." The dashboard shows worker
status (active/idle/rate-limited) but not whether the fleet is making forward
progress. A worker can be "active" for 2 hours on a single issue with no board
movement.

**Rust fix**: the Orchestrator emits a periodic `FleetProgress` event:

```rust
struct FleetProgress {
    timestamp: DateTime<Utc>,
    active_workers: u32,
    issues_completed_last_hour: u32,
    issues_completed_last_24h: u32,
    stall_violations: Vec<StallViolation>,
    longest_active_issue: Option<(IssueId, Duration)>,
    queue_depth: u32,  // todo items with no worker assigned
}
```

The dashboard renders this as a compact status bar: "5 active, 3 completed/hr, 0
stalls" or "2 active, 0 completed/hr, 3 STALLS" (red). Clicking expands to the full
breakdown.

### L5: Server restart fragility

The Python server re-execs itself on every save of `amux-server.py`. On a shared
checkout with multiple sessions committing, this means uncontrolled restarts. A syntax
error in a commit takes the entire fleet's server down. The server process uses 888MB
RSS and takes 10+ seconds to restart.

**Rust fix**: the compiled binary cannot have syntax errors at runtime. Hot reload is
a `SIGHUP` handler that reloads configuration (`server.env`, gates, schedules) without
restarting the process. The binary is updated via a separate deploy step, not a file
watch. RSS target is <200MB.

### L6: Token waste -- model calls for string manipulation

The Python server makes model API calls (claude -p or Anthropic SDK) for tasks that
should be computed, not inferred:

| Call site | What it does | Tokens/call | Fix |
|---|---|---|---|
| Task title summarizer | `claude -p` to label a board card from a prompt | ~12-15k input | First clause of the prompt IS the title. No model needed. |
| Email event extractor | Haiku to parse event emails | ~3k input | Structured parsing with regex + date parser. Model call only for ambiguous cases. |
| Branch name suggester | Haiku to generate 4 git branch names | ~1k input | Template: `feat/{slug}`, `session/{slug}`, etc. No model needed. |
| Lookup endpoint | Haiku for general "ask Claude" queries | varies | This one is legitimate -- user-facing. |
| browser-use agent | Full Anthropic API call for browser automation | ~4k+ input | Legitimate when doing AI-driven browsing. |

The task summarizer was the worst offender (ethos rule 2: "are you calling the model
for something you could just compute?"). At 12-15k input tokens per call, with 62
sessions each potentially firing one, that is up to 930k tokens per throttle window
for 3-word labels. It was throttled to once per 10 minutes per session, which is why
most commands never reached the board at all -- the throttle was the symptom, not the
fix.

**Rust fix**: no model calls for string manipulation. The title deriver is
`prompt.split('\n')[0].split('.')[0][:80]` -- free, instant, no throttle needed,
every prompt gets a card. Model calls are reserved for judgment: "should this issue
be escalated?", "does this verification evidence satisfy the gate?" -- questions
where the answer depends on understanding, not formatting.

### L7: Browser profile management

Browser automation uses `browser-use` with Chrome profiles for persistent auth state.
The current system has:
- Profile creation via `POST /api/browser/profile/create`
- Profile listing, starting/stopping browser sessions
- Chrome profile path resolution (different between macOS/Linux)
- A bootstrap that patches `browser-use`'s `get_chrome_profile_path` at import time
- Profile cleanup and lock-file management
- Screenshots, CDP integration

Pain points:
- Profile path resolution is fragile (macOS vs Linux, `Default` subdirectory
  inconsistency that caused browser-use to create profiles in the wrong location)
- Browser sessions that don't close properly leave Chrome lock files, blocking the
  next start
- No profile inventory -- you can't see which profiles have which saved logins
- The `browser-use` Python dependency pulls in heavy model deps even when you just
  want profile management

**Rust fix**: browser profiles are a first-class subsystem:

```rust
struct BrowserProfile {
    name: String,
    chrome_data_dir: PathBuf,
    created_at: DateTime<Utc>,
    last_used: Option<DateTime<Utc>>,
    saved_domains: Vec<String>,  // domains with saved auth cookies
    size_bytes: u64,
}

struct BrowserSession {
    profile: String,
    pid: Option<u32>,
    cdp_port: u16,
    started_at: DateTime<Utc>,
    screenshots: Vec<PathBuf>,
}
```

- Profile CRUD is native (no Python browser-use dependency for management)
- Chrome is launched directly via CDP flags, not through a Python wrapper
- Lock files are cleaned up on server start (reconciliation)
- Profile inventory shows saved auth domains
- Screenshots use CDP directly (`Page.captureScreenshot`)
- The Anthropic model call is separate from the browser control -- you can use
  profiles without burning tokens

### L6: 114 registered sessions, 62 running, 67 with no status

Half the registered sessions are just `.env` files with no running process. The
dashboard shows all 114 with no visual distinction. A user sees 67 blank entries mixed
in with 47 real workers.

**Rust fix**: workers that are stopped are in a collapsed "Stopped" section by
default. The main view shows only running + recently-active workers. The worker count
in the header shows "6 active / 41 idle / 67 stopped" -- three numbers, not one.

## What does NOT change

- SQLite as the single data store (same schema, same file, directly compatible)
- Self-signed TLS on port 8822
- `~/.amux/` directory structure
- `server.env` config mechanism
- API route paths and response shapes (dashboard compatibility)
- The ethos: the harness gets better as the models get better

## What changes

- `sessions` -> `workers` everywhere
- Implicit orchestrator -> explicit Orchestrator with typed assignments and leases
- Flat board -> dependency graph with typed relations
- String-based scope -> three-tier Global/Group/Worker with deterministic inheritance
- Terminal scraping as control plane -> OpenCode structured agent protocol for
  commands/events/lifecycle, with herdr/tmux/native PTY as process hosts
- tmux as sole backend -> herdr primary process host, tmux fallback, native PTY future
- Implicit agent interaction -> OpenCode for structured semantics, herdr for hosting
- `done` as final state -> `done` (worker claim) vs `verified` (harness conclusion)
- 30 Python threads -> single tokio select! loop + spawned tasks
- Port doc -> system invariant doc with behavioral acceptance tests
