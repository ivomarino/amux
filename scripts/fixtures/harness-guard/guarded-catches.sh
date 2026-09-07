#!/usr/bin/env bash
# FIXTURE (AF-561): `set -e` AND a call to a helper that does not exist.
# The DYNAMIC positive control: this must exit 127 and print NO verdict. Without
# it the meta-test only proves it can read a `set` line, not that the guard those
# lines buy actually stops a false PASS.
set -euo pipefail
PASS=0; FAIL=0
ok() { PASS=$((PASS+1)); }
ok "a real cell"
check_not "a cell whose helper does not exist" "x" "y"
echo "fixture: $PASS passed, $FAIL failed"
