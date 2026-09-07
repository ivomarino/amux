#!/usr/bin/env bash
# FIXTURE (AF-561): carries `command_not_found_handle` and NOT `set -e`.
# This is the thing that SHIPPED BROKEN on 2026-09-07 (40938593, reverted by
# ab50fe1d): the handler needs bash >= 4.0 and this box runs 3.2.57, so it never
# fires and the suite still prints a verdict. The meta-test MUST reject it —
# accepting it would certify an inert guard as protection.
set -uo pipefail
PASS=0; FAIL=0
command_not_found_handle() { FAIL=$((FAIL+1)); return 127; }
ok() { PASS=$((PASS+1)); }
ok "a real cell"
check_not "a cell whose helper does not exist" "x" "y"
echo "PASS ($PASS cells)"
