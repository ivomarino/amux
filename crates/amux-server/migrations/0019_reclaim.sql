-- Disk reclaim: scans, findings and the quarantine ledger.
--
-- Why a quarantine table rather than just deleting: a size heuristic that is
-- wrong about ONE directory costs a restore click instead of the user's data
-- (ethos rule 8 — never bulk-delete user content on an agent's judgment).
--
-- Why `df_free_before`/`df_free_after` are on the batch and not derived from
-- summing `bytes`: on APFS the two genuinely disagree. Deleting 22GB of ollama
-- blobs moved `du` by 22GB and moved `df` by ~0, because 24 hourly local
-- snapshots still referenced the blocks. A UI that reports the SUM would have
-- claimed a reclaim the user cannot see on their own disk, which is the
-- loud-wrong-instrument failure in ethos rule 7. Record what the filesystem
-- actually gave back.

CREATE TABLE IF NOT EXISTS reclaim_scans (
  id            TEXT PRIMARY KEY,
  started_at    INTEGER NOT NULL,
  finished_at   INTEGER,
  status        TEXT NOT NULL DEFAULT 'running',  -- running|done|failed|cancelled
  roots         TEXT NOT NULL DEFAULT '[]',       -- JSON array of scanned roots
  error         TEXT,
  -- progress, updated in place while running so the UI can show real motion
  dirs_walked   INTEGER NOT NULL DEFAULT 0,
  files_walked  INTEGER NOT NULL DEFAULT 0,
  bytes_seen    INTEGER NOT NULL DEFAULT 0,
  current_path  TEXT,
  -- volume state captured at scan time
  df_total      INTEGER,
  df_free       INTEGER,
  snapshot_count INTEGER,
  session       TEXT
);

CREATE INDEX IF NOT EXISTS idx_reclaim_scans_started ON reclaim_scans(started_at DESC);

CREATE TABLE IF NOT EXISTS reclaim_findings (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  scan_id     TEXT NOT NULL,
  category    TEXT NOT NULL,   -- build|cache|devtool|stale|duplicate|snapshot
  path        TEXT NOT NULL,
  bytes       INTEGER NOT NULL DEFAULT 0,
  file_count  INTEGER NOT NULL DEFAULT 0,
  mtime       INTEGER,         -- newest mtime under path (staleness signal)
  regenerable INTEGER NOT NULL DEFAULT 0,  -- 1 = rebuilding costs time, not data
  detail      TEXT,
  UNIQUE(scan_id, path)
);

CREATE INDEX IF NOT EXISTS idx_reclaim_findings_scan ON reclaim_findings(scan_id, bytes DESC);

-- Treemap nodes: the folder tree with rolled-up sizes, kept separate from
-- findings because the map shows EVERYTHING (including what must never be
-- deleted) while findings are only the actionable subset.
CREATE TABLE IF NOT EXISTS reclaim_tree (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  scan_id   TEXT NOT NULL,
  path      TEXT NOT NULL,
  parent    TEXT,
  depth     INTEGER NOT NULL DEFAULT 0,
  bytes     INTEGER NOT NULL DEFAULT 0,
  file_count INTEGER NOT NULL DEFAULT 0,
  kind      TEXT,             -- dominant file-type bucket under this node
  UNIQUE(scan_id, path)
);

CREATE INDEX IF NOT EXISTS idx_reclaim_tree_scan ON reclaim_tree(scan_id, parent, bytes DESC);

CREATE TABLE IF NOT EXISTS reclaim_quarantine (
  id             TEXT PRIMARY KEY,
  created_at     INTEGER NOT NULL,
  scan_id        TEXT,
  status         TEXT NOT NULL DEFAULT 'staged',  -- staged|restored|purged|failed
  item_count     INTEGER NOT NULL DEFAULT 0,
  bytes          INTEGER NOT NULL DEFAULT 0,
  df_free_before INTEGER,
  df_free_after  INTEGER,
  purged_at      INTEGER,
  session        TEXT,
  error          TEXT
);

CREATE TABLE IF NOT EXISTS reclaim_quarantine_items (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  batch_id      TEXT NOT NULL,
  original_path TEXT NOT NULL,
  staged_path   TEXT NOT NULL,
  bytes         INTEGER NOT NULL DEFAULT 0,
  category      TEXT,
  status        TEXT NOT NULL DEFAULT 'staged',   -- staged|restored|purged|failed
  error         TEXT
);

CREATE INDEX IF NOT EXISTS idx_reclaim_qitems_batch ON reclaim_quarantine_items(batch_id);
