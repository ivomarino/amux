# AMUX-4225: health stalls amplified into redundant build/adoption

AMUX-4220 commit `9bc77193a08c4d1d1e1da76c298c0724055d4c03` was
already reachable from origin/main and the running image when this continuation
began. Its Codex attribution RCA and the September 7 tmux recovery evidence
remain intact. No worker restart or signal to protected tmux PIDs was used.

## Evidence (September 8, EDT)

- 09:56:41: git-branch probe timed out in child_exit after 5.08s; recorded server
  CPU 97.7%, git children in U state. This proves contemporaneous host/process
  contention, not the initiating cause of every subsequent stall.
- 10:03:18: watchdog health timeout, recovery 10:03:57.
- 10:09:40: unannounced server start, PID 91066. Watchdog did not record a
  restart-threshold firing then. The initiating stop remains unproven.
- 10:12:56: builder began rebuilding already-stamped `a604412b492330dc67e4138b0b77443840fa72b9`,
  logging matching health commit `a604412b4923` as STAMP DRIFT. Build took 3m01s.
- 10:14:50 and 10:15:40: watchdog timed out; 10:16:06: same-revision self-exec;
  watchdog recovered 10:16:10.
- 10:22:16: another same-revision build entered debug-cache cleanup (18GB);
  cleanup failed with Directory not empty.
- 10:26:27 and 10:28:37: TLS-handshake health timeouts; recoveries 10:26:57 and
  10:29:07. A genuinely new `3e98c1fd` build began 10:26:41, installed, and was
  followed by another health-unmeasured STAMP DRIFT/overlap-guard failure.
- Owner monitor: 10:34:xx sessions request returned zero bytes before a 20s
  timeout. TubeScience independently timed out registering handoffs. Health
  answered normally again at 10:35:25. These stalls affect worker coordination.

Private receipts are under `~/.amux/logs/amux-4225/`, including health/build
brackets, board reconciliation, original builder test failure, and fleet reads.
The native macOS sample returned a report with an empty call graph: stacks were
**unmeasured**, not evidence of an idle server.

## Confirmed defects and correction

The builder already compared full SHAs with abbreviated prefixes. Its first two
health curls could fail, while a THIRD independent curl succeeded only when
formatting the drift log. The build decision and its stated evidence therefore
came from different measurements. A failed probe also authorized a rebuild of
an already installed revision, increasing CPU/IO load during an outage.

The builder now probes once under its global lock, validates the identity,
and defers on unavailable/invalid evidence. Matching full/abbreviated SHAs skip;
a hash-verified elected install awaiting adoption skips recompilation. An actual
foreign overwrite still triggers repair. Atomic install writes a full-SHA,
content-hash receipt; identical bytes retain the existing executable inode.
The adopter verifies receipt against bytes and skips identical binaries and
same-revision rebuilds. Boot and adoption logs carry PID, source and image identity.

Health performed two synchronous pool acquisitions on a Tokio worker (each can
wait 30 seconds), allowing fleet reads to delay health and unrelated runtime
work. It now uses one nonwaiting connection on a blocking thread, a 250ms HTTP
probe deadline, and one in-flight probe per store. Failure returns HTTP 503 with
image identity and an unmeasured board result; the log names phase and reason.
This is a reproduced failure path, not proof it caused every recorded TLS stall.

Registered periodic/loop jobs now log individual polls exceeding 250ms. This
names synchronous work occupying an async runtime worker while excluding time
spent awaiting IO. It supplies the missing attribution for the next stall.

## Focused proof before publication

- `scripts/safe-cargo.sh check --workspace`: finished dev profile, exit 0.
- `scripts/test-contended.sh -p amux-server --test health_alias`: 2 passed,
  including exhausted-pool health identity/runtime responsiveness and recovery.
- `bash scripts/test-build-activation-authority.sh`: 39 passed, 0 failed.
  The timeout-then-matching-answer regression against the original script
  produced 27 passed, 10 failed (downstream build counts also fail).
- `bash scripts/test-build-atomic-install.sh`: 5 passed, 0 failed.
- Live sessions read: 54 non-archived, 54 running. Protected tmux process start
  times unchanged. Full publication gates and post-adoption measurements are
  recorded on the card when they finish; these focused results are not a claim
  that the entire historical stall has been explained.
