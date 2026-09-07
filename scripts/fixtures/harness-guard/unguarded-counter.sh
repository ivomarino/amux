#!/usr/bin/env bash
# FIXTURE (AF-561): unguarded, `PASS=0; FAIL=0` counter idiom — the COMMON one here.
# A detector built only on `echo "PASS` finds 3 of 43 real harnesses and would call
# this file clean. The meta-test MUST reject it.
set -uo pipefail
PASS=0; FAIL=0
ok() { PASS=$((PASS+1)); }
ok "a real cell"
check_not "a cell whose helper does not exist" "x" "y"
[ "$FAIL" -eq 0 ] && echo "fixture: $PASS passed, $FAIL failed"
