# AMUX-4220: mvs-research read a sibling's completed Codex turn

## Finding

The idle badge was a real attribution defect. The invariant detected the
disagreement but blamed an ignored stop-hook report instead of preserving the
signal that actually decided the status.

On September 8, 2026, the retained invariant rows contain 26 failures for
`mvs-research` from 04:59:13 through 05:23:31 EDT. The incident's cumulative
occurrence count is 31; its first occurrence dates to August 21. This diagnosis
establishes the September 8 failure, not the cause of every older occurrence.

`mvs-research` and `mvs-pitr` share a working directory. Their worker start
timestamps were 22:04:12 and 22:04:14 UTC on September 7. The Codex rollouts' first
`session_meta` records were timestamped 22:09:20.757 and 22:09:19.439 respectively.
Both workers therefore selected pitr's rollout under the previous rule: choose
the rollout nearest the worker's start within a 15-minute window.

Kernel evidence settles the identity independently of that heuristic:

| Worker | Pane PID | Native Codex PID | Open rollout UUID |
| --- | ---: | ---: | --- |
| mvs-research | 65447 | 10061 | 01a07de8-a61c-7490-ad4c-409220892555 |
| mvs-pitr | 64968 | 9275 | 01a07de8-980b-7342-becd-52c8c2933d31 |

`tmux list-panes`, the `ps` parent chain, and `lsof` agree on these associations.
Research's rollout records its board-task turn from 04:58:28 to 05:11:47 EDT,
followed by another turn from 05:12:23 to 05:24:08. Pitr's latest completion was
04:10:18. Amux reported research idle from that older **sibling** completion
while research was working. Its 30-hour-old stop-hook report was both from a
previous worker life and unapplied.

At investigation time research was actually idle, with its completed board
task visible above the Codex prompt. It had no current per-entity invariant
evaluation because the monitor captures recently painting panes. That absence
is not a fresh pass and does not invalidate the retained failures.

## Change

- Startup association refuses multiple matching rollouts instead of selecting
  the nearest. Subagent rollouts cannot supply the main worker's turn boundary.
  Uniqueness examines the full file population, so the previous 80-file cutoff
  cannot hide a quiet sibling. A short header cache avoids reopening all
  historical headers for every lane; turn events remain uncached.
- Explicit `codex_session_id` remains the first choice. If its file is missing,
  resolution refuses to substitute a sibling.
- Deduplicated `status_truth` warnings publish `ambiguous_worker_rollout` or
  `claimed_rollout_missing`, measurement fields, and the candidate filenames.
  Unresolved association falls back to existing terminal/activity signals; it
  does not manufacture a structured completion.
- Status explanations include the selected rollout filename. The invariant
  preserves that explanation and uses the deciding Codex boundary's age for
  the transition grace period. It no longer labels every mismatch as a hook
  report overriding physical evidence.

The recovery uses the existing session identity field and the kernel-proven
rollout, preserving the running process and conversation. Private receipts,
including the original invariant evidence, status history, pane, and process
ownership, live in `~/.amux/logs/amux-4220/`. The board card carries final test,
commit, and live-adoption results.

This is separate from the initiating tmux stall investigation in
[the September 7 incident](2026-09-07-tmux-socket-takeover.md). That evidence and
AMUX-4203's remaining scope are preserved; this task does not close that RCA.
