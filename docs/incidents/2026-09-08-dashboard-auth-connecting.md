# Dashboard remains on Connecting after authorization refusals

Card: AMUX-4246. Observation: 2026-09-08, 20:08–20:10 EDT.

The owner screenshot showed an empty Workers page, a Connecting spinner, a
Polling badge, and a Clear cache link. The exact browser origin was not supplied.
The request log independently records sustained unauthorized requests from one
remote client; this is consistent with the screenshot, not a proven browser ID.

## Observed evidence

At 20:10 EDT, `/api/health` answered in 34 ms at commit
`99f53c8c6662449982cfd13fb06993a5ec911162`, build `501298a174acab44`, PID 91066.
`/api/debug/sse?since_h=1` reported measured=true, n_considered=13,
live_connections=3, opened_total=3. These observations establish a responding
server and other live clients; they do not prove every client path is healthy.

`/api/logs/analyze?since_h=1` reported measured=true, n_considered=3960:

- 777 GET `/api/sessions` responses were 401, between 19:10:53 and 20:10:01 EDT.
- The same remote client had 777 refusals each for board, statuses and session gates.
- 45 delta-sync requests and 38 POST `/api/client-debug` reports were also 401.
- Responses were immediate JSON authorization refusals, not read timeouts.

Private raw snapshots are retained under `~/.amux/logs/amux-connecting-20260908/`.
The previous health/build-churn incident and its worker-preservation evidence
remain separate. No fleet restart or tmux signal is part of this repair.

## Root cause and evidence limit

`fetchSessions` parsed the response then returned on a non-array payload. A 401
therefore neither cleared initial loading nor created a visible failure state.
The connectivity badge inferred Polling from the absence of SSE, regardless of
whether the worker request was authorized. Error objects from other HTTP statuses
and malformed successful responses had the same silent-loading edge.

The old recovery function unconditionally assumed a stale owner bearer and
requested a service-worker update followed by reload. Within one app version the
SW could serve the same cached anonymous bootstrap. A genuinely absent or revoked
login also cannot be repaired by reloading. The old 401 body did not record which
credential path failed, and client-debug itself required authentication. That
historical evidence cannot distinguish missing, invalid, or revoked credentials.

## Repair

Worker reads now resolve into data or a visible failure. Authorization refusals
show Access required, a recovery action, and optional connection details. Saved
workers remain explicitly marked as a saved copy. HTTP/payload failures show Sync
error. Only successful worker data clears the error; unrelated SSE traffic does
not convert a failed worker read into a healthy badge.

One bounded bootstrap refresh can repair a missing/stale cached owner bearer.
The existing server credential rules must supply a different valid bearer before
the client reloads through the SW's `_fresh` bypass. The complete bootstrap is
reloaded so its UI guard changes with the bearer. An anonymous or revoked-member
shell cannot gain owner access through this recovery.

The server's 401 JSON now names the refusal and recovery path, so request-log
analysis can explain failures even while diagnostic beacons are rejected.
`dashboard_auth_rejected` logs the reason and credential-presence booleans;
`dashboard_bootstrap_*` records the shell's access decision. No credential values
or query strings are included. A bounded latest-failure record survives in browser
session storage and is sent to client-debug after a successful worker read.

Regression coverage: remote-style missing credentials with/without cached workers,
retry to a genuinely empty fleet, deferred diagnostics, server errors and invalid
payloads, real isolated-server recovery from missing/stale cached credentials,
and server credential admission/refusal cases. Desktop, 375px Chromium and iOS
Safari exercise the visible UI. The pre-fix assets must fail the new 401 test.
