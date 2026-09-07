# Task graph contract

amux's task graph is a typed, rebuildable projection of its existing board,
workers, filesystem artifacts and messages. SQLite remains authoritative. The
graph cannot independently assign work, move a card, rewrite evidence or create
another copy of a task. Better models can author better decompositions through
the same primitives; the harness checks structure and records evidence.

## Use it

```bash
amux board graph                 # summary and every integrity finding
amux board graph --check         # exit 0 valid; 1 invalid; 2 unavailable
amux board graph --json > task-graph.json
```

The full export API is `GET /api/graph/board`. Routine CLI summaries and checks
use `GET /api/graph/board/verify`, which reads only structural fields and omits
task prose and artifact payloads. Both use the same verifier. The full export
was 58 MB on the live board; repeatedly downloading it just to validate a plan
would make the graph expensive to use. Both routes appear in `/api/debug/routes` and
`/api/board/contract`. The response covers **all non-deleted tasks**, including
archived and terminal tasks: filtering these out would invent dangling edges.
An invalid graph still returns its findings and surviving components. A failed
read returns HTTP 500, `measured:false`, `n_considered:0`, and `why_unmeasured`.

Author a plan with `amux board decompose <capture-id> --stdin`, edit dependencies
through the revision-checked board PATCH, set parentage with `amux board epic`,
and register outputs with `amux board artifact`. These APIs keep their existing
attribution, transaction, history, gate and evidence contracts. The Map editor's
free-form graph rows are not the task graph.

## Structure and provenance

Schema version 1 uses namespaced, stable node IDs (`task:AMUX-42`,
`worker:backend`, `artifact:<registry-id>`, `message:<history-id>`). The semantic
source ID is also preserved in `key`. Node kinds and relation kinds are Rust
enums; extensible metadata lives in `attributes`.

| Directed relation | Source of truth | Meaning |
|---|---|---|
| task → task `depends_on` | `issues.depends_on` | The source requires the target |
| task → task `part_of` | `issues.epic` | The source belongs to the target parent |
| task → worker `assigned_to` | `issues.session` | Recorded assignment |
| task → artifact `produces` | `_amux_task_artifacts.task_id` | Registered output association |
| task → message `recorded_message` | `cmd_history.card_id` | Recorded message association |

Each edge includes its source table, record ID, field, and task revision where
available. Message association does not imply that the message created the
task: callbacks and later messages can also name it. Worker nodes do not claim
the worker is currently running. Artifact references preserve the actual URL,
file path or commit; `existence_measured:false` prevents a recorded path being
mistaken for an existence check. Artifact lifecycle state remains separate from
task completion and verification state.

Task nodes expose descriptions, continuation, acceptance criteria, evidence,
source references, and the detail API URL for the authoritative history. Only
recorded relationships are projected; prose citations do not become edges.

## Verification and build order

The verifier checks raw dependency encoding, duplicate dependencies, missing or
deleted task references, dependency cycles, and parent cycles. Dependency and
parent DAGs are checked separately: a parent may depend on its child's result
while that child belongs to the parent. Combining those edges creates a false
cycle in a perfectly normal decomposition.

Dependency `layers` give a deterministic prerequisite-first order. A layer's
members have no dependency on one another. Cyclic components, missing targets,
malformed tasks, and affected dependents are excluded from these layers and
listed in `unbuildable`. `cycles` names actual strongly connected components,
not innocent dependents trapped behind them. The iterative algorithms handle
deep plans without recursive stack overflow.

**Structural order is not permission to execute.** Task status, configured
workflow columns, gates, ownership, waits and WIP still determine runtime
readiness through the existing ready frontier and claim API. Completing a
prerequisite and satisfying a verification gate remain distinct actions.

## Verified dependency handoff

`amux board request <producer> <title> --for <original-task>` creates a real
dependency and arms a durable return callback by default. The original task
releases its doing slot and stays queued while the prerequisite is unresolved.
Code, ops, blocker and tripwire dependencies require Verified; types whose
workflow ends at Done resolve there. Missing, deleted and discarded prerequisites
are not successful delivery. If the result is no longer required, explicitly
remove or replace the relationship rather than claiming it was verified.

Claims, the ready frontier, backlog promotion and completion callbacks use the
same resolution predicate. A callback names the producer, prerequisite, original
task, recorded outcome, next action and any remaining blockers. When the return
task is unique, the Messages row links to that original task. Both cards retain
the indicator in their histories. The dependency edge remains after resolution
as provenance, and no longer blocks execution.

The callback uses the existing persistent steering outbox and stable
`task-callback-<dependency-id>` delivery key. Restart recovery cannot duplicate
the indicator. Delivery occurs at the receiving worker's next available turn
boundary; it cannot interrupt an in-flight tool. A reopened dependency with an
undispatched callback is held for verification again. Discard sends a failure
indicator, with remaining blockers explicitly named. Isolated workers keep their
existing delivery restriction, which is recorded as a visible refusal.

Sweep signals: `dependency_waiting_for_verification`,
`dependency_callback_linked`, `dependency_callback_not_resolved`,
`dependency_callback_graph_unmeasured`, and `dependency_resolution_unmeasured`.

Dependency writes check reachability back to the edited task inside the writer
transaction. An unrelated old cycle cannot mask a newly introduced cycle or
block an unrelated valid edit. Parent writes independently reject cyclic
lineage. Refusals log `dependency_cycle_rejected` or `lineage_cycle_rejected`.
The periodic `board.graph_integrity` invariant checks legacy/bypass corruption
without fetching task prose or artifact payloads; the graph endpoint logs
`task_graph_invalid` or `task_graph_unmeasured` as appropriate.

## Snapshot identity and extension

One read transaction captures the global revision, nodes, edges and validation.
Nodes, edges, findings and dependency layers are sorted deterministically. The
`sha256` hashes the canonical serialized tuple `(nodes, edges, findings)`;
revision and observation time are excluded from that content digest. Repeating
the read over unchanged source content yields the same hash, while changing an
artifact state changes it. The hash identifies content; it is not a signature
or proof that work was performed. Save the JSON with test artifacts when a
historical snapshot must remain available after source-record retention.

To extend the graph, add a typed relation and a projection from an existing
primitive, record its source fields, define whether it participates in a DAG,
and add a negative control. Additive attributes are compatible; consumers must
tolerate additive kinds. A change of meaning or identity requires a new schema
version. Source migrations remain ordinary board/primitive migrations. There
is no separate graph migration, synchronization worker or graph database.

## Research context and design decisions

- [Karpathy's autoresearch](https://github.com/karpathy/autoresearch) and its
  [experiment instructions](https://github.com/karpathy/autoresearch/blob/master/program.md)
  use bounded experiments, fixed evaluation and commit-linked result records.
  They do not prescribe a general task-graph architecture. The useful lesson
  here is executable verification tied to durable evidence.
- [Anthropic's long-running harness work](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
  emphasizes incremental work, explicit feature state, progress artifacts and
  testing across context windows. amux already has those primitives; the graph
  makes their relationships inspectable.
- [LangGraph persistence](https://docs.langchain.com/oss/python/langgraph/persistence)
  distinguishes thread checkpoints from longer-lived shared stores. This
  supports keeping provider execution state distinct from amux's durable task
  state. This change does not claim to add provider checkpoint replay.
- [W3C PROV-DM](https://www.w3.org/TR/prov-dm/) distinguishes entities,
  activities, responsible agents and derivation. That informs explicit typed
  provenance here; the API does not claim PROV interchange compliance.

These are design inferences applied to amux, not a claim that one graph
framework is universally best. New model capabilities should improve planning
and evidence quality without requiring a replacement control plane.
