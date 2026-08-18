-- .mdai computed-file engine run history (AMUX-3240).
--
-- Append-only log of every DAG node opened during a run: which file, when, the
-- hash of its resolved inputs (resolved sources + body prompt + model), the
-- output it produced, the model used, and whether the output was reused from
-- cache. The cache is an input-hash short-circuit: a node whose inputs are
-- unchanged since its last run reuses that output instead of spending a model
-- call to reproduce an identical result (ethos rule 2: do not call the model
-- for what you can compute).
--
-- Why the `cached` column exists rather than being inferred: a reused output
-- and a freshly computed one are otherwise byte-identical rows, so "did this
-- open actually spend a model call?" would be unanswerable from the log alone
-- (ethos rule 4: a diagnosis that is impossible from the kept data IS the bug).
-- With it, a run that silently stopped recomputing (a cache that is wrong) is
-- visible in the history, not just felt.
CREATE TABLE IF NOT EXISTS mdai_runs (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  path        TEXT NOT NULL,               -- root-relative path of the .mdai node
  ts          INTEGER NOT NULL,            -- unix seconds when the node was opened
  inputs_hash TEXT NOT NULL,               -- sha256 of (model + body + resolved sources)
  output      TEXT NOT NULL,               -- the produced (or reused) node output
  model       TEXT NOT NULL,               -- resolved model id used for this node
  cached      INTEGER NOT NULL DEFAULT 0,  -- 1 = reused prior output, no model call
  session     TEXT                         -- X-Amux-Session that triggered the run, if any
);

-- Newest-first history per file: the run endpoint and the cache both read the
-- most recent row for a path, so index (path, id DESC).
CREATE INDEX IF NOT EXISTS idx_mdai_runs_path ON mdai_runs(path, id DESC);
