#!/usr/bin/env bash
# FIXTURE (AF-561): guarded with `set -e`. The meta-test MUST accept this one,
# or it is a check that rejects everything and proves nothing.
set -euo pipefail
PASS=0; FAIL=0
ok() { PASS=$((PASS+1)); }
ok "a real cell"
echo "fixture: $PASS passed, $FAIL failed"
