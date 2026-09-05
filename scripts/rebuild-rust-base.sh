#!/usr/bin/env bash
# Rebuild the amux-rust-base image on every Woodpecker build host, from
# this repo's own Dockerfile.rust-base.
#
# OWNERSHIP: amux, not infra (Ivo's call, 2026-09-05) — infra provisioned
# the 3 build hosts themselves (build-02.baar, build.home, build.virt04)
# and their Woodpecker server/agents, and still owns THAT layer, but the
# actual amux-rust-base image (its Dockerfile lives here, its rebuild
# cadence is driven by amux's own Cargo.lock) is this repo's job now.
# Before this, infra ran the build via three Tofu `docker_image` resources
# (one per host) as an incidental part of standing up each build host —
# see infra's own tofu/docker-build-{02-baar,home,northstage}/
# amux-rust-base.tf for that prior mechanism, kept in place only until
# this script has been verified working at least once against all three
# hosts (avoids a gap where nobody rebuilds the image).
#
# WHY A SCRIPT, NOT A THIRD TOFU MODULE HERE: this repo has no IaC of its
# own and no reason to grow one for a single docker_image resource --
# infra's own docker-mtls PKI is what actually reaches each host's
# dockerd, and that's infra's credential to hand out, not amux's state to
# own. This script is deliberately the same shape as the existing
# rust-remote-build.sh (docker --context <host> build), just fanned out
# over every build host instead of one.
#
# Requires, per host, EXACTLY the transport infra's docker-mtls module
# already set up (see infra's tofu/docker-mtls/ -- TCP + mutual TLS,
# tlsverify, never plain SSH, never 0.0.0.0):
#   - a `docker context` already created for it, e.g.:
#       docker context create build-02-baar \
#         --docker "host=tcp://<host-ip>:2376,ca=<ca.pem>,cert=<cert.pem>,key=<key.pem>"
#   - listed (space-separated context names) in AMUX_RUST_BASE_CONTEXTS,
#     e.g. in this box's own ~/.amux/server.env (private, gitignored --
#     see CLAUDE.local.md, this repo is public and neither hostnames nor
#     cert paths belong in it).
#
# No hostnames/IPs hardcoded here on purpose (same reasoning as
# rust-remote-build.sh) -- context names and their connection details are
# this box's own local Docker CLI config, not repo content.
#
# Each host is built independently and a slow/failed host does not block
# the others -- build-02.baar in particular is documented (infra's own
# CLAUDE.md) to have a flaky/slow uplink; this must not turn a fleet-wide
# rebuild into an all-or-nothing operation blocked on that one host.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOCKERFILE="Dockerfile.rust-base"

CONTEXTS="${AMUX_RUST_BASE_CONTEXTS:-}"
if [ -z "$CONTEXTS" ]; then
  echo "AMUX_RUST_BASE_CONTEXTS not set -- space-separated docker context" \
       "names, one per build host (see this script's own header, and" \
       "CLAUDE.local.md for this box's real values)" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker not on PATH" >&2
  exit 1
fi

LOGDIR="$(mktemp -d)"
declare -A STATUS
PIDS=()
CTX_BY_PID=()

for ctx in $CONTEXTS; do
  (
    if ! docker --context "$ctx" info >/dev/null 2>&1; then
      echo "context '$ctx' unreachable" >&2
      exit 1
    fi
    docker --context "$ctx" build \
      -t amux-rust-base:latest \
      -f "$REPO/$DOCKERFILE" \
      "$REPO"
  ) >"$LOGDIR/$ctx.log" 2>&1 &
  PIDS+=($!)
  CTX_BY_PID+=("$ctx")
done

fail=0
for i in "${!PIDS[@]}"; do
  ctx="${CTX_BY_PID[$i]}"
  if wait "${PIDS[$i]}"; then
    STATUS[$ctx]="ok"
  else
    STATUS[$ctx]="FAILED"
    fail=1
  fi
done

echo
echo "=== amux-rust-base rebuild summary ==="
for ctx in $CONTEXTS; do
  echo "  $ctx: ${STATUS[$ctx]:-unknown} (log: $LOGDIR/$ctx.log)"
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "one or more hosts failed -- logs kept at $LOGDIR, not cleaned up" >&2
  exit 1
fi

rm -rf "$LOGDIR"
echo "all hosts rebuilt cleanly"
