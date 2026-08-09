# Daily log sweep — the contract (AMUX-2605)

This is a contract for a MODEL, not automation: the sweep is an amux **scheduler**
entry that prompts a session to run these queries, judge the results, and file
board cards. Substrate: `_amux_request_log` (migration 0010), written by
`api/request_log.rs` for every `/api/*` request (exclusions: `/api/events`,
`/api/debug/*`, non-API static paths). `worker` is path-derived
(`/api/sessions/{name}/*`, `/api/workers/{id}/*`), so worker logs are a filter,
never a second log. Retention: `AMUX_REQLOG_RETAIN_DAYS` (default 14).

All queries are GET against the rust origin. `SINCE=$(( $(date +%s) - 86400 ))`.
`total_matched` is the pre-limit count — use it for volumes; never infer volume
from a capped `events` page (limit max 2000).

**COVERAGE (changed 2026-08-09, AF-36 — read this before trusting an all-clear.)**
The table now carries BOTH origins. It used to hold only rust-served requests, and
on 2026-08-09 that was 1,494 rows against python's 129,940 in the same window —
1.1% of traffic — so the sweep reported "0 5xx, 0 auth failures, no latency
outliers" while 52 x 400, 3 x 401, 5 x 403 and a 3.3s board GET sat unseen on the
other origin. Every sweep below was correctly specified and structurally blind.
Discriminate with `answered_by`: `native` = rust, `python` = python origin,
`python-proxy` = proxied through. If a sweep ever returns zero 400s across a whole
day again, check `SELECT answered_by, COUNT(*)` before believing it — a
single-origin result is the tell that coverage regressed.

## The five sweeps, in order

1. **Errors, grouped by family.**
   `GET /api/logs?since=$SINCE&min_status=400&limit=2000`
   Group by `family` yourself; read `error_body`(`resp`), `worker`, `amux_session`.
   Finding = any family with a new error shape, or an error count out of line
   with its own norm. 401/404 noise from probes is not a finding; a 500 is
   always a finding.

2. **Latency p95 outliers vs the family's trailing norm.** Per busy family:
   `GET /api/logs?since=$SINCE&family=/api/board&limit=2000` -> p95 of `latency_ms`;
   compare against the trailing week: same query with `since=$(( $(date +%s) - 691200 ))`.
   Finding = today's p95 > ~2x trailing p95 (use judgment on low-volume families;
   never conclude from n < 20 requests).

3. **Proxy volume — must trend to zero post-cutover.**
   `GET /api/logs?since=$SINCE&answered_by=python-proxy&limit=1` -> `total_matched`.
   Record the number. Finding = it rose vs yesterday, or any family outside
   `GET /api/debug/boundary`'s `proxied` list shows proxy rows (a boundary
   regression).

4. **401/403 spikes by client IP.**
   `GET /api/logs?since=$SINCE&min_status=401&limit=2000`, keep status 401/403,
   group by `ip`. Finding = any non-loopback IP with a burst (>20/day), or a
   loopback caller failing auth repeatedly (a broken token on a lane).

5. **Worker traffic with no board trace.** Collect distinct `worker` values from
   `GET /api/logs?since=$SINCE&limit=2000`; cross-check `GET /api/board` for cards
   with that session updated in the window. Finding = a worker generating real
   API traffic whose board shows nothing in `doing`/updated — silent work
   (task-ledger rule violation), or a runaway loop hammering the API.

Also skim `GET /api/logs/raw?lines=500` for `sources:"server_log"` lines matching
ERROR/WARN — the tracing tail carries failures that never became a request row.

## Triage rule (mandatory)

Every finding becomes ONE board card (`amux board add --stdin`), containing:
the finding in one line, the exact query that found it (verbatim URL), the
numbers (count/p95/IP), and the suspected family/worker. No umbrella cards.
Nothing found = no cards and no message; do not file "sweep ran" noise.
