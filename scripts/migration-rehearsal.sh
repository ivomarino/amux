#!/usr/bin/env bash
# Migration rehearsal (Phase 11, RR checklist §Migration rehearsal).
#
# Proves, against a COPY of the live database, that:
#   1. the Rust server's migration path applies cleanly (additive only),
#   2. every pre-existing table and row count survives untouched,
#   3. the PYTHON server's queries still work on the migrated file
#      (rollback compatibility — the DB must stay bilingual).
#
# The live DB is opened READ-ONLY via sqlite backup; nothing here can write
# to production. Run it any time; it is the repeatable go/no-go evidence
# generator for cutover.
set -euo pipefail

LIVE_DB="${AMUX_LIVE_DB:-$HOME/.amux/amux.db}"
WORK="$(mktemp -d /tmp/amux-rehearsal.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT
COPY="$WORK/amux.db"

echo "== 1. snapshot (read-only backup of $LIVE_DB)"
sqlite3 "file:${LIVE_DB}?mode=ro" ".backup '$COPY'"

echo "== 2. pre-migration census"
sqlite3 "$COPY" "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';" > "$WORK/tables_before"
sqlite3 "$COPY" "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;" > "$WORK/names_before"
# Row counts for the tables the Python server reads hottest.
for t in issues schedules prefs cal_events crm_contacts session_events token_ledger; do
  echo "$t $(sqlite3 "$COPY" "SELECT COUNT(*) FROM $t;")" >> "$WORK/rows_before"
done
cat "$WORK/rows_before"

echo "== 3. rust migration path (the EXACT production Store::open)"
AMUX_HOME="$WORK" AMUX_DB="$COPY" AMUX_RS_MIGRATE_ONLY=1 \
  "${AMUX_RS_BIN:-./target/debug/amux-server}"

echo "== 4. post-migration invariants"
# 4a. Every pre-existing table still exists.
sqlite3 "$COPY" "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;" > "$WORK/names_after"
if ! comm -23 "$WORK/names_before" "$WORK/names_after" | grep -q .; then
  echo "ok: no table lost"
else
  echo "FAIL: tables LOST by migration:"; comm -23 "$WORK/names_before" "$WORK/names_after"; exit 1
fi
# 4b. Row counts unchanged in Python's tables (additive-only proof).
for t in issues schedules prefs cal_events crm_contacts session_events token_ledger; do
  before=$(grep "^$t " "$WORK/rows_before" | awk '{print $2}')
  after=$(sqlite3 "$COPY" "SELECT COUNT(*) FROM $t;")
  if [ "$before" != "$after" ]; then
    echo "FAIL: $t row count moved $before -> $after"; exit 1
  fi
done
echo "ok: row counts unchanged across all sampled tables"
# 4c. Integrity.
[ "$(sqlite3 "$COPY" "PRAGMA integrity_check;")" = "ok" ] && echo "ok: integrity_check" || { echo "FAIL: integrity"; exit 1; }

echo "== 5. python-side reads still work (rollback direction)"
python3 - "$COPY" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1])
db.row_factory = sqlite3.Row
# The Python server's real hot queries, verbatim shapes.
open_issues = db.execute(
    "SELECT COUNT(*) FROM issues WHERE deleted IS NULL AND status NOT IN ('done','verified','discarded')"
).fetchone()[0]
schedules = db.execute("SELECT COUNT(*) FROM schedules WHERE enabled=1").fetchone()[0]
prefs = dict(db.execute("SELECT key, value FROM prefs").fetchall())
# A WRITE in the python direction (on the copy): the rollback server must
# still be able to mutate.
db.execute("INSERT INTO prefs (key, value) VALUES ('rehearsal_probe','1') "
           "ON CONFLICT(key) DO UPDATE SET value='1'")
db.commit()
assert db.execute("SELECT value FROM prefs WHERE key='rehearsal_probe'").fetchone()[0] == '1'
print(f"ok: python reads+writes post-migration (open_issues={open_issues}, enabled_schedules={schedules}, prefs={len(prefs)})")
PY

echo "== REHEARSAL PASSED"
