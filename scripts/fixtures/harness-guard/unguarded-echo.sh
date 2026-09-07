#!/usr/bin/env bash
# FIXTURE (AF-561): unguarded, `echo "PASS` idiom. The meta-test MUST reject this.
set -uo pipefail
CELLS=0
check() { CELLS=$((CELLS + 1)); echo "  ok    $1"; }
check "a real cell"
check_not "a cell whose helper does not exist" "x" "y"
echo "PASS ($CELLS outcome cells)"
