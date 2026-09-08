-- Production harness enforcement and feedback plane.
--
-- Additive only: the Python-compatible baseline remains readable.  These
-- tables make the already-present harness concepts durable at the point they
-- are enforced: capability receipts, checkpoints/handoffs, task budgets,
-- sensor profiles, guide-rule lifecycle, tool observations, and fleet state.

-- ADDCOL: _amux_memories expires_at TEXT
-- ADDCOL: _amux_memories last_validated_at TEXT
-- ADDCOL: _amux_memories superseded_by TEXT

-- ADDCOL: _amux_context_snapshots checkpoint_version INTEGER
-- ADDCOL: _amux_context_snapshots harness_version TEXT

-- ADDCOL: _amux_verifications criteria_version INTEGER NOT NULL DEFAULT 0
-- ADDCOL: _amux_verifications harness_version TEXT
-- ADDCOL: _amux_verifications duration_ms INTEGER NOT NULL DEFAULT 0

CREATE TABLE IF NOT EXISTS _amux_policy_receipts (
    id              TEXT PRIMARY KEY,
    actor           TEXT NOT NULL,
    action          TEXT NOT NULL,
    resource        TEXT NOT NULL,
    trust           TEXT NOT NULL,
    reversible      INTEGER NOT NULL,
    effect          TEXT NOT NULL,
    rule_id         TEXT,
    rationale       TEXT NOT NULL,
    cost_microusd   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_policy_receipts_created
    ON _amux_policy_receipts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_amux_policy_receipts_actor_action
    ON _amux_policy_receipts(actor, action, created_at DESC);

CREATE TABLE IF NOT EXISTS _amux_policy_approvals (
    token_hash      TEXT PRIMARY KEY,
    actor           TEXT NOT NULL,
    action          TEXT NOT NULL,
    resource        TEXT NOT NULL,
    approved_by     TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    used_at         TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_policy_approvals_expiry
    ON _amux_policy_approvals(expires_at, used_at);

CREATE TABLE IF NOT EXISTS _amux_task_budgets (
    task_id         TEXT PRIMARY KEY,
    limits          TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS _amux_task_checkpoints (
    task_id         TEXT PRIMARY KEY,
    version         INTEGER NOT NULL DEFAULT 1,
    completed_steps TEXT NOT NULL DEFAULT '[]',
    next_action     TEXT NOT NULL,
    artifacts       TEXT NOT NULL DEFAULT '[]',
    unresolved      TEXT NOT NULL DEFAULT '[]',
    input_hash      TEXT NOT NULL,
    context_hash    TEXT,
    last_result     TEXT,
    updated_by      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS _amux_handoffs (
    id                  TEXT PRIMARY KEY,
    task_id             TEXT NOT NULL,
    assignment_key      TEXT,
    objective           TEXT NOT NULL,
    criteria_version    INTEGER NOT NULL DEFAULT 0,
    checkpoint_version  INTEGER,
    checkpoint_hash     TEXT,
    artifacts           TEXT NOT NULL DEFAULT '[]',
    evidence            TEXT NOT NULL DEFAULT '[]',
    assumptions         TEXT NOT NULL DEFAULT '[]',
    unresolved          TEXT NOT NULL DEFAULT '[]',
    next_action         TEXT NOT NULL,
    deadline            TEXT,
    sender              TEXT NOT NULL,
    receiver            TEXT NOT NULL,
    created_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_handoffs_task
    ON _amux_handoffs(task_id, created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_amux_handoffs_assignment
    ON _amux_handoffs(assignment_key) WHERE assignment_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS _amux_sensor_profiles (
    task_type       TEXT PRIMARY KEY,
    criteria        TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS _amux_harness_rules (
    id                  TEXT PRIMARY KEY,
    scope               TEXT NOT NULL,
    name                TEXT NOT NULL,
    content             TEXT NOT NULL,
    owner               TEXT NOT NULL,
    source_failure      TEXT NOT NULL,
    rationale           TEXT NOT NULL,
    status              TEXT NOT NULL,
    enforcement_layer   TEXT NOT NULL,
    sensor_ref          TEXT,
    version             INTEGER NOT NULL DEFAULT 1,
    added_at            TEXT NOT NULL,
    last_validated_at   TEXT,
    expires_at          TEXT,
    superseded_by       TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_amux_harness_rules_scope_name_live
    ON _amux_harness_rules(scope, name)
    WHERE status IN ('candidate', 'active');
CREATE INDEX IF NOT EXISTS idx_amux_harness_rules_status
    ON _amux_harness_rules(status, enforcement_layer);

CREATE TABLE IF NOT EXISTS _amux_tool_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    worker_id   TEXT NOT NULL,
    task_id     TEXT,
    turn_id     TEXT,
    tool        TEXT NOT NULL,
    detail      TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_tool_events_task
    ON _amux_tool_events(task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_amux_tool_events_worker
    ON _amux_tool_events(worker_id, created_at);

CREATE TABLE IF NOT EXISTS _amux_fleet_state (
    singleton   INTEGER PRIMARY KEY CHECK (singleton = 1),
    state       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS _amux_decompositions (
    parent_task_id  TEXT PRIMARY KEY,
    child_task_id   TEXT NOT NULL,
    depth           INTEGER NOT NULL,
    status          TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
