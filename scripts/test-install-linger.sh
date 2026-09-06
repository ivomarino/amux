#!/usr/bin/env bash
# test-install-linger.sh — AF-527: install.sh must say when lingering is OFF.
#
# Every unit install.sh writes on Linux is a systemd USER unit with
# WantedBy=default.target, so without lingering the whole set is inert on a
# headless box: nothing starts at boot, everything stops at logout. Issue #92's
# reporter hit exactly this and hand-wrote a /etc/systemd/system unit instead.
#
# The function is EXTRACTED FROM install.sh rather than restated here, so the
# test cannot pass against a copy that has drifted from what ships.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
FAILS=0
fail() { echo "FAIL: $1" >&2; FAILS=$((FAILS + 1)); }

# Pull the shipped function out of install.sh. If it is renamed or deleted this
# extraction yields nothing and every case below fails loudly, which is the
# point — a check that silently tests an empty string is worse than no check.
awk '/^  linger_advice\(\) \{/,/^  \}/' "$ROOT/install.sh" | sed 's/^  //' > "$T/fn.sh"
if ! grep -q 'loginctl' "$T/fn.sh"; then
  fail "could not extract linger_advice() from install.sh — has it been renamed or removed?"
  echo "$FAILS check(s) failed" >&2; exit 1
fi

stub_loginctl() {  # $1 = what `--property=Linger --value` should print
  cat > "$T/bin/loginctl" <<EOS
#!/usr/bin/env bash
printf '%s\n' "$1"
EOS
  chmod +x "$T/bin/loginctl"
}
mkdir -p "$T/bin"

run_fn() { ( PATH="$T/bin:$PATH"; . "$T/fn.sh"; linger_advice ); }

# ---- CASE 1: lingering OFF -> must warn, must name the exact command, rc=1
stub_loginctl "no"
OUT=$(run_fn 2>&1); RC=$?
if [ "$RC" -ne 0 ]; then echo "  ok   linger off is reported as a problem (rc=$RC)"
else fail "linger OFF returned rc=0 — the caller cannot tell, so nothing is printed at the end"; fi
if printf '%s' "$OUT" | grep -q 'loginctl enable-linger'; then
  echo "  ok   it names the one command that fixes it"
else
  fail "the warning does not name 'loginctl enable-linger' — that omission IS the bug (issue #92)"
fi
if printf '%s' "$OUT" | grep -q 'NOT start at boot'; then
  echo "  ok   it says what actually breaks, not just that a setting is off"
else fail "the warning does not say what goes wrong"; fi

# ---- CASE 2 (CONTROL): lingering ON -> silent, rc=0. Without this the check
# could warn unconditionally and case 1 would still pass.
stub_loginctl "yes"
OUT2=$(run_fn 2>&1); RC2=$?
if [ "$RC2" -eq 0 ] && [ -z "$OUT2" ]; then
  echo "  ok   lingering already on is silent (the control)"
else
  fail "warned even though lingering is ON (rc=$RC2, out='$OUT2') — an unconditional warning trains people to skip it"
fi

# ---- CASE 3 (CONTROL): no loginctl at all -> must not fail the install.
rm -f "$T/bin/loginctl"
OUT3=$( ( PATH="$T/bin:/usr/bin:/bin"; . "$T/fn.sh"; linger_advice ) 2>&1 ); RC3=$?
if [ "$RC3" -eq 0 ]; then echo "  ok   absent loginctl is not treated as a failure"
else fail "a box without loginctl gets a spurious warning (rc=$RC3)"; fi

if [ "$FAILS" -eq 0 ]; then echo "ok: install linger advice — all 5 checks pass"; else echo "$FAILS check(s) failed" >&2; fi
exit $((FAILS > 0))
