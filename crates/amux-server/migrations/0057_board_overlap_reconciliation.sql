-- Durable, concern-scoped coordination for two workers that discover they
-- overlap after both have begun.  The header's primary key is the atomic
-- election: one semantic concern has exactly one elected owner, while the
-- same semantic work may deliberately retain distinct concerns side by side.
--
-- This belongs to the board rather than provider conversation state.  A
-- worker can vanish, compact, or switch model/provider without losing either
-- the election, the reciprocal card links, or the callback that still needs
-- delivery.

CREATE TABLE IF NOT EXISTS board_overlap_coordination (
    coordination_id TEXT PRIMARY KEY,
    semantic_key TEXT NOT NULL,
    concern TEXT NOT NULL,
    owner_card_id TEXT NOT NULL,
    owner_session TEXT NOT NULL,
    resolution TEXT NOT NULL DEFAULT 'pending'
        CHECK(resolution IN ('pending', 'merged', 'scope-split', 'released')),
    resolution_note TEXT,
    resolved_by_card_id TEXT,
    resolved_by_session TEXT,
    resolved_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(semantic_key, concern)
);

CREATE TABLE IF NOT EXISTS board_overlap_members (
    coordination_id TEXT NOT NULL,
    card_id TEXT NOT NULL,
    session TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    head_commit TEXT NOT NULL,
    worktree TEXT NOT NULL,
    intent TEXT NOT NULL,
    self_reported INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    PRIMARY KEY(coordination_id, card_id),
    FOREIGN KEY(coordination_id) REFERENCES board_overlap_coordination(coordination_id)
);

CREATE TABLE IF NOT EXISTS board_overlap_refs (
    coordination_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('evidence', 'asset')),
    ref_value TEXT NOT NULL,
    card_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(coordination_id, kind, ref_value, card_id),
    FOREIGN KEY(coordination_id) REFERENCES board_overlap_coordination(coordination_id)
);

CREATE TABLE IF NOT EXISTS board_overlap_callbacks (
    coordination_id TEXT NOT NULL,
    target_session TEXT NOT NULL,
    message_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK(state IN ('pending', 'queued', 'retryable', 'delivered')),
    error TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(coordination_id, target_session),
    UNIQUE(message_id),
    FOREIGN KEY(coordination_id) REFERENCES board_overlap_coordination(coordination_id)
);

CREATE INDEX IF NOT EXISTS idx_board_overlap_semantic
    ON board_overlap_coordination(semantic_key, concern);
CREATE INDEX IF NOT EXISTS idx_board_overlap_callback_pending
    ON board_overlap_callbacks(state, updated_at)
    WHERE state IN ('pending', 'retryable');

-- Defense in depth for every writer, including a future code path that does
-- not go through the HTTP PATCH guard.  A linked non-owner may not declare
-- Done/Verified while this concern remains pending (or has been merged into
-- the elected owner's work).  Only an explicit `scope-split` or `released`
-- reconciliation clears the block.
CREATE TRIGGER IF NOT EXISTS board_overlap_peer_completion_guard
BEFORE UPDATE OF status ON issues
WHEN NEW.status IN ('done', 'verified')
 AND NEW.status <> OLD.status
 AND EXISTS (
    SELECT 1
      FROM board_overlap_members m
      JOIN board_overlap_coordination c ON c.coordination_id = m.coordination_id
     WHERE m.card_id = NEW.id
       AND m.card_id <> c.owner_card_id
       AND c.resolution NOT IN ('scope-split', 'released')
 )
BEGIN
    SELECT RAISE(ABORT, 'board overlap peer completion blocked; reconcile or scope-split first');
END;
