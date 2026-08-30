-- AF-319: close the loop on the idle-backlog drain nudge.
--
-- MEASURED over 34 hours to 2026-08-30: 419 of 988 fleet messages (42%) are
-- board_drive nudges, 12.3/hour, 574k characters (~143k tokens) pushed into
-- worker contexts. Per lane, against card movements in the SAME window:
--
--   mvs-infra     80 nudges (22 min apart)  ->  0 cards moved
--   backend       56 nudges (36 min)        ->  0
--   byo-ray       41 nudges (46 min)        ->  0
--   mixpeek-cicd  38 nudges (54 min)        ->  0
--   ts-gke        25 nudges (84 min)        ->  0
--   amux          18 nudges                 -> 82
--   amux-frustr.  24 nudges                 -> 40
--
-- Every one of those lanes was running, not credit-limited and not waiting. So
-- 240 nudges to the five hardest-nudged lanes moved nothing, while the two
-- LEAST-nudged lanes did all the work. Nudge frequency is anti-correlated with
-- the outcome it exists to produce.
--
-- The cause is a missing feedback term, not a wrong constant.
-- `idle_backlog_drain_cooldown_s` scales cadence UP with backlog SIZE (base 2h,
-- halving every ~25 cards, 20m floor). Frequency is therefore a function of
-- backlog, and backlog is not a function of frequency, so the largest backlogs
-- saturate at the floor and stay there forever.
--
-- This table is the missing term, and it is a TABLE rather than a HashMap
-- because the auto-builder restarts this process on every commit: in-memory
-- state would reset several times an hour and the cap would never be reached
-- (the D1 "in-memory state is fiction" deviation).
CREATE TABLE IF NOT EXISTS board_drive_nudge_state (
    session         TEXT PRIMARY KEY,
    -- Consecutive drain nudges that were followed by no card movement at all.
    unheeded        INTEGER NOT NULL DEFAULT 0,
    last_nudge_at   REAL,
    -- MAX(issues.updated) for this lane when we last nudged. The next tick
    -- compares against it: anything larger means a card moved.
    board_mark      INTEGER,
    -- The escalation card filed when we gave up nudging, so we file ONE and
    -- not one per tick.
    escalated_card  TEXT
);
