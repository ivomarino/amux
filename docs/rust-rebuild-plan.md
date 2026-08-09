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
- **3 terminal backends**: tmux (primary), herdr, iTerm2
- **4 LLM providers**: Claude Code (OAuth + API key), Gemini, Codex, Ollama
- **Full SPA dashboard** with SSE real-time updates, PWA/offline, dark/light themes

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

A **backend** is the process host: tmux, herdr, native PTY, or anything else. It
spawns, kills, captures, and sends -- but does not observe or decide.

```
Worker (durable entity)
 ├── Session 1 (ran, hit context limit, ended)
 ├── Session 2 (ran, crashed via OOM, ended)
 └── Session 3 (currently running)
       └── Backend: tmux pane "worker-name"
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
    Archive,
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
and events internally; tmux/herdr translate at the boundary.

```rust
enum WorkerCommand {
    ExecuteIssue(IssueId),
    Continue,
    Steer { text: String },
    Verify(IssueId),
    Review(IssueId),
    Cancel,
    Pause,
    Resume,
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

This is the actual D1 exit. tmux scraping becomes an adapter that emits
`WorkerEvent::RateLimited` instead of the orchestrator matching regexes.

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

```rust
enum TerminalStatus {
    Verified,
    Archived,
    Discarded,
    BlockedByUser { reason: String },
}

// The orchestrator runs this check on every tick:
fn stall_check(worker: &Worker, board: &Board) -> Vec<StallViolation> {
    if worker.state != WorkerState::Idle { return vec![]; }
    board.issues_for_worker(worker.id)
        .filter(|i| !i.status.is_terminal())
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
2. If the issue is blocked by a dependency: no stall (it is waiting for another issue)
3. If the worker has no runnable issues left in its scope: escalate to the group or
   reassign
4. Otherwise: the worker MUST be given the issue and told to continue

This is tested in every Playwright golden scenario: at the end of every test, assert
that no worker is idle with non-terminal issues. A stall is a CI failure.

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
    // SSE emitted immediately -- dashboard sees the change within 1 event cycle
    emit_sse(Event::WorkerStateChanged { id: worker.id, state: worker.state });
}
```

For hooks-capable providers (Claude Code), events come from hooks in real time. For
hookless providers (Gemini, Codex, Ollama), the terminal adapter polls and translates
to WorkerEvents -- but the adapter owns the translation, not the orchestrator. The
consumer code is identical either way.

**Per-provider event coverage** (what each provider can report today):

| Event | Claude (hooks) | Claude (scrape) | Gemini | Codex | Ollama |
|---|---|---|---|---|---|
| TurnStarted | UserPromptSubmit hook | regex | regex | regex | regex |
| TurnCompleted | Stop hook | regex | regex | regex | regex |
| RateLimited | -- | 14 regex patterns | 2 patterns | 1 pattern | 1 pattern |
| ContextLow | -- | regex | -- | -- | -- |
| Failed (crash) | -- | process check | process check | process check | process check |

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
  queuedAt: number;        // timestamp
  retries: number;         // retry count
  lastError?: string;      // last sync error
  optimisticState?: any;   // local state applied before server confirmed
}

interface CachedState {
  workers: Worker[];       // last-known worker list
  board: BoardItem[];      // last-known board state (desc-truncated)
  workerDetails: Map<string, WorkerDetail>;  // peek/history per worker
  lastSync: number;        // when server state was last confirmed
  serverRev: number;       // server's state revision (for delta sync)
}
```

#### Server-side support

```rust
// Every mutating endpoint accepts an idempotency key
#[derive(Deserialize)]
struct MutationRequest<T> {
    #[serde(flatten)]
    payload: T,
    #[serde(default)]
    idempotency_key: Option<Uuid>,
}

// Idempotency table: stores results of completed mutations for 24h
// A replayed operation with the same key returns the cached result
// without re-executing

// Delta sync: client sends its last-known revision, server returns
// only what changed since then
#[derive(Serialize)]
struct SyncResponse {
    rev: u64,
    workers: Option<Vec<Worker>>,        // None = unchanged
    board_changes: Vec<BoardDelta>,      // additions, updates, deletions since rev
    pending_commands: Vec<PendingCmd>,    // commands queued for this client
}

enum BoardDelta {
    Upsert(BoardItem),
    Delete { id: IssueId },
    StatusChange { id: IssueId, from: Status, to: Status },
}
```

#### Conflict resolution

Conflicts are surfaced, never swallowed:

| Conflict type | Resolution |
|---|---|
| Board card moved by someone else while offline | Toast with both states, user picks |
| Board card deleted while offline edit queued | Toast: "card was deleted", discard edit |
| Worker command sent while worker stopped | Toast: "worker stopped", offer restart |
| Stale optimistic state (server rev moved) | Silent merge if non-conflicting, toast if conflicting |

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

**Test plan**:
- Playwright: go offline, create board card, send worker command, go online, verify
  both applied
- Playwright: go offline, queue 5 commands, go online, verify all 5 replay in order
- Playwright: offline queue + conflict (another user moved the card) shows toast
- Playwright: dashboard renders all tabs from service worker cache while offline
- Playwright: delta sync -- make 3 board changes on server, client reconnects,
  receives only the 3 deltas (not full board reload)
- Playwright: idempotency -- replay same operation twice, verify no duplicate
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
   orchestrator, board, scheduler, and dashboard never know whether tmux or herdr is
   running, or whether Claude or Ollama is the provider. If a feature requires
   `if provider == "claude"` above the adapter, the adapter's interface is wrong.

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

Every backend implementation passes the same test suite unchanged. Every provider
adapter passes the same test suite unchanged.

```rust
// One suite, runs against Mock, Tmux, Herdr, NativePty:
mod backend_conformance {
    async fn test_spawn_and_capture(backend: &dyn SessionBackend);
    async fn test_send_text_roundtrip(backend: &dyn SessionBackend);
    async fn test_interrupt(backend: &dyn SessionBackend);
    async fn test_terminate(backend: &dyn SessionBackend);
    async fn test_crash_recovery(backend: &dyn SessionBackend);
    async fn test_restart_reconciliation(backend: &dyn SessionBackend);
    async fn test_large_prompt(backend: &dyn SessionBackend);
    async fn test_unicode(backend: &dyn SessionBackend);
    async fn test_multiline(backend: &dyn SessionBackend);
    async fn test_concurrent_workers(backend: &dyn SessionBackend);
    async fn test_backend_disappears(backend: &dyn SessionBackend);
}
```

**The invariant: no test above the backend/provider layer knows which
backend/provider is running.**

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

### Invariant 28: Cheapest verifier first

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

```rust
enum VerifierKind {
    Command { cmd: String, expected_exit: i32 },
    HttpCheck { url: Url, expected_status: u16 },
    FileExists { path: PathBuf },
    PlaywrightAssertion { script: String },
    ModelJudgment { prompt: String },
    HumanReview,
}
```

Verifiers run in cost order. If the free checks fail, expensive ones never run.

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
   PROVIDER          BACKEND
 Claude/Gemini/    tmux/herdr/
 Codex/Ollama      native PTY
      |                |
      +-------+--------+
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
       │                        │ execution
       │                 ┌──────▼───────┐
       │                 │Backend (tmux/│
       │                 │herdr/native) │
       │                 └──────────────┘
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

The terminal backend is the dominant complexity center: ~90-100 tmux subprocess call
sites, ~50 compiled regexes, 5 polling loops at 2s/3s/15s/60s/60s intervals, ~700
lines for rate-limit detection alone.

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
| **B: herdr primary** | Structured `agent prompt/read` | Send quality, alt-screen capture | Same scraping regexes |
| **C: Native PTY** | Rust owns PTY via `portable-pty` | Zero subprocess overhead, streaming | Must solve persistence, lose manual attach |
| **D: Structured protocol** | WorkerCommand/WorkerEvent, hooks as primary | IS the D1 exit | Depends on Claude Code hook coverage |

### Recommendation

**tmux as initial backend + WorkerCommand/WorkerEvent as the internal protocol (A + D).**

The key insight: **tmux is not the problem -- the scraping is.** Changing which process
hosts the PTY does not eliminate D1. The WorkerCommand/WorkerEvent protocol does, by
making the terminal backend an adapter that translates PTY text into typed events. As
Claude Code's hook coverage grows, the adapter shrinks. herdr stays as an optional
backend behind the trait. Native PTY (Option C) is a future target once hooks cover
enough that the scraper is liveness-only.

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
          workers.rs             # /api/workers/*
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
          auth.rs                # bearer token, share tokens, org
          sse.rs                 # /api/events
          health.rs              # /health
          static_files.rs        # embedded dashboard
        orchestrator/
          mod.rs                 # runtime orchestrator loop
          reconcile.rs           # startup reconciliation
          pickup.rs              # runnable-issue selection
          context.rs             # context assembly pipeline
        runtime/
          mod.rs                 # job scheduling (DurableSchedule vs PeriodicTask)
          scheduler.rs           # user-facing durable schedules
          periodic.rs            # internal maintenance tasks
        backend/
          mod.rs                 # SessionBackend trait
          tmux.rs                # tmux subprocess (initial default)
          herdr.rs               # herdr agent backend
          adapter.rs             # terminal output -> WorkerEvent translator
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

### Key dependencies

| Concern | Crate | Notes |
|---|---|---|
| HTTP server | `axum` | async, tower middleware |
| Async runtime | `tokio` | multi-threaded, timers, process, signal |
| SQLite | `rusqlite` + `r2d2` | `bundled` feature, WAL mode, single-writer task |
| JSON | `serde` + `serde_json` | derive-based |
| SSE | `axum::response::sse` | built-in |
| TLS | `rustls` + `rcgen` | self-signed cert |
| Subprocess | `tokio::process` | tmux, git, node, browser-use |
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
   Provider -- all types, no I/O. This is the system's vocabulary.
3. `amux-server/db`: all 47 tables as SQL migrations, WAL mode, single-writer task
4. `amux-server/config`: three-tier config loading (global/group/worker), `server.env`
5. `amux-server/api`: axum router, static file embedding, `/health`, auth
6. TLS setup with self-signed cert
7. **Golden scenario test harness** (Playwright-based): end-to-end scenario tests
   that will run against every phase. Start with:
   - Server starts, dashboard loads, health returns 200
   - Auth rejects bad token, accepts good token

**Test plan**:
- Unit: scope resolver merges global < group < worker correctly, worker wins conflicts
- Unit: scope resolver with group gates overriding global gates
- Unit: scope resolver with worker env overriding group env
- Unit: all 47 tables created in in-memory DB
- Unit: `BoardTransition` state machine rejects invalid transitions
- Unit: API request/response types match OpenAPI spec (generated from JsonSchema derives)
- Unit: `DurableEvent` append succeeds for every `EventKind` variant (Invariant 24)
- Unit: backpressure -- bounded channels reject/drop correctly at capacity (Invariant 26)
- Unit: `ContextFragment` priority ordering is deterministic (Invariant 16)
- Unit: `GateEvaluator::Deterministic` runs before `Model` (Invariant 28)
- Simulation: fake clock + fake backend, orchestrator tick completes in <1ms (Invariant 22)
- Simulation: deterministic replay of 100 random event sequences produces identical state
- proptest: `BoardTransition` state machine rejects all invalid (from, to) pairs (Invariant 22)
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
3. `amux-server/backend/tmux.rs`: SessionBackend impl for tmux
4. `amux-server/backend/adapter.rs`: terminal output -> WorkerEvent translator (port
   ANSI stripping, prompt detection, rate-limit regexes, per-provider patterns)
5. `amux-server/api/workers.rs`: CRUD, start (202 async), stop, peek, send
6. `amux-server/orchestrator`: runtime loop, startup reconciliation
7. SSE: worker state stream

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
- Simulation: 50 workers, 200 issues, fake clock -- orchestrator assigns optimally with
  no double-leases (Invariant 22)
- Simulation: provider rate-limit + recovery -- fleet redistributes within 2 ticks (Invariant 20)
- Simulation: worker crash mid-issue -- lease reclaimed, issue re-assigned (Invariant 22)
- proptest: no double-lease for arbitrary event sequences (Invariant 22)
- proptest: verified implies done occurred previously (Invariant 22)
- Backend conformance: tmux passes full suite (Invariant 21)
- Provider conformance: Claude adapter passes full suite (Invariant 21)
- Integration: create Claude worker, start, verify tmux session, send text, capture
- Integration: create Ollama worker (`ollama run` backend), start, verify running
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

**Goal**: WorkerCommand delivery and WorkerEvent processing.

1. WorkerCommand -> terminal adapter (steering delivery with dedup, idempotency)
2. Terminal adapter -> WorkerEvent (rate-limit detection per provider, crash detection)
3. Scan demotion: hook-reported workers get demoted capture frequency
4. Auto-responder for `--dangerously-skip-permissions` workers
5. Turn tracking: TurnStarted/TurnCompleted events drive the orchestrator

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
- Integration: enqueue command, verify delivery within 4s
- Integration: rate-limit auto-wait fires on simulated terminal output
- Integration: `IntegrationState` transitions reflected in `/health` endpoint (Invariant 23)
- Integration: Gmail unavailable -> email operations queue, recover on reconnect (Invariant 23)
- Playwright: worker status updates live in dashboard, rate-limit shown within 2s
- Playwright: provider quota dashboard shows fleet-level capacity (Invariant 20)

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

**Playwright golden scenarios (the acceptance criteria)**:

Each scenario runs end-to-end in a real browser. Timing is measured and asserted.

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
- Playwright: all dashboard tabs render, SSE updates live, PWA offline works

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
- Performance: all latency targets met under load (40 workers, 100 board items)
- Performance: RSS stays flat over 24h soak test
- Performance: no file descriptor leaks over 24h

### Phase 10: CI/CD pipeline (est. 1 week)

**Goal**: zero-regression guarantee.

Pipeline stages:
1. `cargo check` + `clippy` -- compile-time correctness
2. `cargo test` -- all unit + integration + simulation + proptest tests
3. Backend conformance suite -- MockBackend + TmuxBackend (Invariant 21)
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

#### Rollback plan

At any point during shadow or swap:
1. Stop Rust server
2. Start Python server on 8822
3. DB is compatible in both directions (no destructive migrations)

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
3. **Terminal scraping**: 50+ regexes must be ported and tested. Mitigation: extract
   test corpus from Python, run as unit tests per provider.
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

### L2: tmux target format inconsistency

The `=` prefix for exact session matching works for session-level commands
(`has-session`, `kill-session`) but silently fails for pane-level commands
(`capture-pane`, `send-keys`, `pipe-pane`) -- they need `=name:` with a trailing
colon. This caused a fleet-wide outage: every capture and send-keys across 62 sessions
was silently failing. The test only verified `has-session` and `kill-session`.

**Rust fix**: the `SessionBackend` trait encapsulates all tmux interaction. The target
format is derived once, tested once, used everywhere. No raw `subprocess::Command`
construction outside the backend module. The test harness exercises every tmux verb the
backend uses, not just the ones that motivated the fix.

```rust
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
enum StallReason {
    WorkerIdle,                          // worker has capacity but isn't assigned
    WorkerRateLimited { reset_at: DateTime },  // waiting, not stalled
    DependencyBlocked { blocked_by: IssueId }, // not stalled, just waiting
    NoCapableWorker,                     // no worker can do this work
    BackendFailure { error: String },    // capture/send broken
    GateBlocked { gate: Vec<String> },   // needs human ack
    Orphaned,                            // assigned to a worker that no longer exists
}
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
- Terminal scraping as control plane -> WorkerCommand/WorkerEvent protocol with
  terminal as adapter
- `done` as final state -> `done` (worker claim) vs `verified` (harness conclusion)
- 30 Python threads -> single tokio select! loop + spawned tasks
- Port doc -> system invariant doc with behavioral acceptance tests
