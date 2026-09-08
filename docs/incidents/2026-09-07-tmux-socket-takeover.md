# Workers disappeared behind a replaced tmux socket

AMUX-4203. All times below are EDT on 2026-09-07.

At 17:27:23, amux began logging concurrent tmux capture timeouts. At
17:29:49, `list-sessions` reported `no server running` (ECONNREFUSED).
At 17:30:07, a replacement tmux server started on the default socket.
The original server remained alive, holding 62 sessions. Amux's dashboard
and invariants only saw the replacement server, initially with seven workers.
This was lost access to the original fleet, not evidence of 62 process exits.

Two independent observations established the split: `lsof` showed two
live tmux PIDs holding sockets named at the same default path, while
`tmux display-message -p '#{pid}|#{socket_path}'` identified only the newer
PID as reachable. Preserving the replacement socket, sending the original
server SIGUSR1, and preserving its recreated socket at a separate path
restored access to all 62 original sessions. The default path was restored
to the replacement server after 22ms; neither process was killed by this probe.

Tmux can unlink and recreate a socket during implicit server creation after
ECONNREFUSED. Its `-N` flag prevents that implicit creation. See the upstream
[client connection path](https://github.com/tmux/tmux/blob/3.6a/client.c) and
[socket recreation and SIGUSR1 handling](https://github.com/tmux/tmux/blob/3.6a/server.c).
The observed replacement followed connection refusals. We did not retain a
syscall trace proving which client removed the old directory entry.

## Recovery

The owner requested that every non-archived worker resume its previous work.
We captured 200 terminal lines from each original registered worker, then
retired only the 57 original non-archived worker copies on the recovered
socket. Eight replacement workers were already running and were preserved.
The remaining 49 were started through amux's resume path. Three needed an
individual retry after launch setup failed under load. The final observed
population was **57 registered non-archived workers, 57 running**.

Continuation instructions were queued for the other 56 workers; the amux
worker continued this incident investigation. Idle workers whose messages
remained queued were moved to direct owner delivery. When the API stalled,
we submitted those instructions through their existing tmux panes and kept
local delivery receipts. Active workers retained
their instructions for their next turn boundary. Archived workers were not
selected for restart. The five unregistered sessions on the original server
were left intact rather than being assumed disposable.

Local receipts, original pane captures, start results, continuation IDs and
before/after health snapshots are under
`~/.amux/logs/worker-recovery-20260907/`. Raw captures are not committed because
they contain private worker content.

## What is known, and what is not

The original tmux stall's initiating cause is **not established**. There was
no contemporaneous stack sample or socket-owner record in amux's diagnostics.
A later tmux sample during recovery showed substantial time processing
`capture-pane`, consistent with capture load contributing to stalls, but
that is not proof of what happened at 17:27. The server watchdog also restarted
amux during recovery; it did not destroy the replacement tmux sessions.

A separate startup defect was directly visible in a recovered terminal:
`unset ANTHROPIC_API_KEYclaude ...`. Startup sent a literal line and Enter
through separate tmux clients, ignored failed submissions, and continued
with the next command. The fix submits the literal and Enter in a single
tmux command queue and logs a failed submission without logging command text.

## Prevention and evidence

- Existing-server starts use `tmux -N`, including the bash CLI, legacy API,
  and typed worker backend. A refused connection may create a server only
  after socket-owner enumeration establishes that no live process owns that
  path. Missing ownership measurement is a refusal, not an empty result.
- `/api/debug/tmux` includes independently measured socket ownership,
  responding PID, probe failures and explicit verdicts. Its fleet query is
  bounded so the diagnostic itself cannot wait indefinitely on a stalled tmux.
- `session.tmux_socket_has_one_live_owner` reports unreachable live owners
  and multiple owners through existing invariant storage and WARN logs.
- An unreachable live owner or multiple owners trigger a durable JSON receipt with process
  statistics and, on macOS, a stack sample. Sample success/failure is recorded.
  Full process arguments and environment are deliberately not collected.

The recovery alias can leave multiple owners reporting the original pathname;
that remains visible until those original unregistered sessions are retired.
A responsive replacement alone is not proof that the older owner has exited.

## Verification

- `bash -n amux` → exit 0.
- `python3 scripts/test-tmux-start-guard.py` → `Ran 4 tests`, `OK`.
  Removing `-N` from an isolated CLI copy made both replacement-prevention
  tests fail, confirming that the regression checks detect the missing guard.
- `CARGO_TARGET_DIR=~/.amux/rust-build-target scripts/safe-cargo.sh check --workspace`
  → `Finished dev profile`.
- `scripts/test-contended.sh -p amux-server tmux_ --lib`
  → `test result: ok. 7 passed; 0 failed`.
- `scripts/test-contended.sh -p amux-server --test tmux_target_audit --test diagnostic_contract`
  → diagnostic contract `4 passed; 0 failed`; tmux target audit `3 passed; 0 failed`.
- Native tmux enumeration joined to the recovery's non-archived worker list
  → `measured: true, n_considered: 57, running: 57, missing: []` at 18:29:21 EDT.

These are targeted checks, not a full-suite result. The contention wrapper
reported worktree changes during the runs; this shared checkout included
concurrent edits. Logs are retained with the local recovery receipts.

## Follow-up: a full connection queue reproduces the takeover

Follow-up work beginning at 22:35 EDT narrowed the mechanism further:

- The request log recorded median host load averages of 54.64 at 17:27,
  144.92 at 17:28, and 157.63 at 17:30. The timeout burst crossed fleet
  discovery, disk probes and background jobs; it was not confined to one pane.
  These are contemporaneous load readings, not attribution to a process.
- Twenty post-recovery stack samples of the replacement tmux server showed
  median capture work of 2.0% and median waiting in `select_dispatch` of 91.7%.
  Ordinary capture traffic in those later samples does not explain the
  original stall. All 57 current pane captures completed when their output
  was drained promptly; none exceeded this host's measured 65,536-byte pipe
  capacity (largest: 16,255 bytes).
- On an isolated tmux server, pausing only that fixture's server and filling
  its connection queue produced **ECONNREFUSED after 128 connections**.
  `tmux -N new-session` refused and preserved the socket. The same command
  without `-N` created a replacement while the original server remained alive.
  `python3 scripts/test-tmux-backlog.py` repeats both controls without touching
  the fleet and prints a measured PASS receipt.

This matches [tmux's 128-entry listen queue](https://github.com/tmux/tmux/blob/3.6a/server.c)
and [macOS's connection-refusal path](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/uipc_usrreq.c).
The incident's missing measurement is still what delayed the original server
and how many connections were queued at that moment. The reproduction proves
a route to takeover; it does not retroactively measure the incident's queue.

The investigation also reproduced a separate defect in amux's bounded probe
runner: both variants waited for child exit before draining piped output.
A child writing 262,144 bytes was killed after three seconds because amux
itself had filled the pipe. Both regression tests failed against the original
runner. The runner now drains stdout and stderr while polling, with the same
deadline covering continuously producing children and descendants that keep
pipes open after the direct child exits.

Timeout WARNs and `/api/debug/tmux` now report the child PID, elapsed time,
drained stdout/stderr byte counts, and whether the wait was for child exit or
pipe EOF. A first timeout schedules independent evidence collection, at most
once per minute, so it does not rely on the busy server runtime's monitor tick.
Receipts include host load and the top processes by CPU and resident memory,
using executable names only. Raw output and process arguments are not logged.
The full-queue and large-output defects are reproduced; neither is claimed as
proof of the initiating cause at 17:27. AMUX-4203 remains open for that evidence.

The owner subsequently requested reconciliation of conflicting Doing claims.
The three prompt-generated duplicates (AMUX-4199, AMUX-4200, AMUX-4202) were
consolidated into AMUX-4203 with links to their preserved requests and recovery
evidence. Fourteen older claims were returned to backlog without declaring
them complete. AMUX-4203 was claimed through the board claim endpoint and
verified as the lane's only Doing card. Before/after board snapshots and action
receipts are retained in the local `followup/` evidence directory.

Follow-up verification:

- Before the pipe fix, `scripts/test-contended.sh -p amux-server bounded_probe_ --lib`
  failed both large-output cases: `0 passed; 2 failed`.
- After the fix, `scripts/test-contended.sh -p amux-server --lib -- bounded_probe_ tmux_host_evidence a_pane_capture_that_never_returns`
  → `test result: ok. 5 passed; 0 failed`.
- `scripts/test-contended.sh -p amux-server --test diagnostic_contract --test tmux_target_audit`
  → diagnostic contract `4 passed; 0 failed`; tmux target audit `3 passed; 0 failed`.
- `scripts/safe-cargo.sh check --workspace` → `Finished dev profile`.
- `python3 scripts/test-tmux-backlog.py` → `measured: true`, `verdict: PASS`,
  128 connections before refusal, guarded socket preserved, unguarded socket replaced.
- Runtime reconciliation → `card_count: 1`, `card_id: AMUX-4203`,
  `source: task.claimed`, `verdict: linked`, `violation: false`.

The wrapper again reported concurrent worktree edits. These targeted results
do not claim a clean full-suite run. The timeout receipt was subsequently
extended to carry the triggering probe's exact byte counts and PID; the commit
gate checks that final production code as well as test targets.
