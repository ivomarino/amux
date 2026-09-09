-- Recursive planning ownership, trace replay, reconciliation, and adaptive
-- throughput controls. All objects extend existing primitives; none replace
-- the board, worker, policy, event, or verification stores.

-- ADDCOL: _amux_handoffs concerns TEXT NOT NULL DEFAULT '[]'
-- ADDCOL: _amux_handoffs deviations TEXT NOT NULL DEFAULT '[]'
-- ADDCOL: _amux_handoffs findings TEXT NOT NULL DEFAULT '[]'
-- ADDCOL: _amux_handoffs requires_replan INTEGER NOT NULL DEFAULT 0
-- ADDCOL: _amux_handoffs planning_scope_id TEXT
-- ADDCOL: _amux_policy_receipts role TEXT

CREATE TABLE IF NOT EXISTS _amux_goal_contracts (
    id                       TEXT PRIMARY KEY,
    objective                TEXT NOT NULL,
    non_goals                TEXT NOT NULL DEFAULT '[]',
    success_metrics          TEXT NOT NULL DEFAULT '[]',
    performance_requirements TEXT NOT NULL DEFAULT '[]',
    resource_constraints     TEXT NOT NULL DEFAULT '[]',
    dependency_policy        TEXT NOT NULL,
    release_policy           TEXT NOT NULL,
    expected_scope_min       INTEGER NOT NULL,
    expected_scope_max       INTEGER NOT NULL,
    version                  INTEGER NOT NULL,
    updated_by               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS _amux_goal_contract_revisions (
    goal_id                  TEXT NOT NULL,
    version                  INTEGER NOT NULL,
    body                     TEXT NOT NULL,
    created_at               TEXT NOT NULL,
    PRIMARY KEY(goal_id, version)
);

CREATE TRIGGER IF NOT EXISTS amux_goal_history_no_update
BEFORE UPDATE ON _amux_goal_contract_revisions
BEGIN SELECT RAISE(ABORT, 'goal contract history is append-only'); END;

CREATE TRIGGER IF NOT EXISTS amux_goal_history_no_delete
BEFORE DELETE ON _amux_goal_contract_revisions
BEGIN SELECT RAISE(ABORT, 'goal contract history is append-only'); END;

CREATE TABLE IF NOT EXISTS _amux_planning_nodes (
    id                   TEXT PRIMARY KEY,
    goal_id              TEXT NOT NULL,
    parent_id            TEXT,
    task_id              TEXT,
    role                 TEXT NOT NULL,
    owner                TEXT NOT NULL,
    title                TEXT NOT NULL,
    objective            TEXT NOT NULL,
    status               TEXT NOT NULL,
    current_plan_version INTEGER NOT NULL DEFAULT 0,
    wake_count           INTEGER NOT NULL DEFAULT 0,
    updated_at           TEXT NOT NULL,
    FOREIGN KEY(goal_id) REFERENCES _amux_goal_contracts(id),
    FOREIGN KEY(parent_id) REFERENCES _amux_planning_nodes(id)
);
CREATE INDEX IF NOT EXISTS idx_amux_planning_nodes_goal
    ON _amux_planning_nodes(goal_id, parent_id);
CREATE INDEX IF NOT EXISTS idx_amux_planning_nodes_owner
    ON _amux_planning_nodes(owner, status);

CREATE TABLE IF NOT EXISTS _amux_current_plans (
    planning_node_id TEXT PRIMARY KEY,
    version          INTEGER NOT NULL,
    body             TEXT NOT NULL,
    reason           TEXT NOT NULL,
    authored_by      TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    FOREIGN KEY(planning_node_id) REFERENCES _amux_planning_nodes(id)
);

CREATE TABLE IF NOT EXISTS _amux_plan_revisions (
    planning_node_id TEXT NOT NULL,
    version          INTEGER NOT NULL,
    body             TEXT NOT NULL,
    reason           TEXT NOT NULL,
    authored_by      TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    PRIMARY KEY(planning_node_id, version),
    FOREIGN KEY(planning_node_id) REFERENCES _amux_planning_nodes(id)
);

CREATE TRIGGER IF NOT EXISTS amux_plan_history_no_update
BEFORE UPDATE ON _amux_plan_revisions
BEGIN SELECT RAISE(ABORT, 'plan history is append-only'); END;

CREATE TRIGGER IF NOT EXISTS amux_plan_history_no_delete
BEFORE DELETE ON _amux_plan_revisions
BEGIN SELECT RAISE(ABORT, 'plan history is append-only'); END;

CREATE TABLE IF NOT EXISTS _amux_turn_traces (
    id              TEXT PRIMARY KEY,
    worker_id       TEXT NOT NULL,
    task_id         TEXT,
    session_id      TEXT,
    turn_id         TEXT NOT NULL,
    kind            TEXT NOT NULL,
    content         TEXT NOT NULL,
    content_sha256  TEXT NOT NULL,
    truncated       INTEGER NOT NULL DEFAULT 0,
    redactions      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_turn_traces_turn
    ON _amux_turn_traces(turn_id, created_at);
CREATE INDEX IF NOT EXISTS idx_amux_turn_traces_created
    ON _amux_turn_traces(created_at);

CREATE TABLE IF NOT EXISTS _amux_reconciliations (
    id              TEXT PRIMARY KEY,
    repo_path       TEXT NOT NULL,
    candidate_sha   TEXT NOT NULL,
    status          TEXT NOT NULL,
    gate_results    TEXT NOT NULL,
    failure_reason  TEXT,
    started_at      TEXT NOT NULL,
    completed_at    TEXT NOT NULL,
    promoted_at     TEXT,
    promoted_by     TEXT
);
CREATE INDEX IF NOT EXISTS idx_amux_reconciliations_repo
    ON _amux_reconciliations(repo_path, completed_at DESC);

CREATE TABLE IF NOT EXISTS _amux_green_snapshots (
    repo_path         TEXT PRIMARY KEY,
    candidate_sha     TEXT NOT NULL,
    reconciliation_id TEXT NOT NULL,
    verified_at       TEXT NOT NULL,
    promoted_at       TEXT,
    promoted_by       TEXT,
    FOREIGN KEY(reconciliation_id) REFERENCES _amux_reconciliations(id)
);

CREATE TABLE IF NOT EXISTS _amux_work_metrics (
    id          TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    source      TEXT NOT NULL,
    duration_ms INTEGER,
    task_id     TEXT,
    detail      TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_amux_work_metrics_kind_time
    ON _amux_work_metrics(kind, created_at);
CREATE INDEX IF NOT EXISTS idx_amux_work_metrics_created
    ON _amux_work_metrics(created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_amux_runtime_queue_observation
    ON _amux_work_metrics(kind, task_id) WHERE kind='queue_wait' AND source='runtime';

CREATE TABLE IF NOT EXISTS _amux_adaptive_wip (
    singleton       INTEGER PRIMARY KEY CHECK(singleton = 1),
    mode            TEXT NOT NULL DEFAULT 'shadow',
    current_limit   INTEGER NOT NULL DEFAULT 1,
    recommended     INTEGER NOT NULL DEFAULT 1,
    min_limit       INTEGER NOT NULL DEFAULT 1,
    max_limit       INTEGER NOT NULL DEFAULT 4,
    sample_size     INTEGER NOT NULL DEFAULT 0,
    evidence_version INTEGER NOT NULL DEFAULT 0,
    reason          TEXT NOT NULL DEFAULT 'insufficient evidence',
    updated_at      TEXT NOT NULL
);
INSERT OR IGNORE INTO _amux_adaptive_wip
    (singleton, mode, current_limit, recommended, min_limit, max_limit,
     sample_size, evidence_version, reason, updated_at)
VALUES
    (1, 'shadow', 1, 1, 1, 4, 0, 0, 'insufficient evidence', '1970-01-01T00:00:00Z');
