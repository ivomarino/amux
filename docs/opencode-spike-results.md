# OpenCode Provider Spike Results (RR-0028e)

## Summary

3 of 4 providers expose structured lifecycle events sufficient for
`OpenCodeAdapter` to serve as the primary agent protocol. The written
branch does NOT fire. `TerminalAdapter` remains the fallback path, not
a peer of `OpenCodeAdapter`. No Phase 1+4 re-estimate required
(RR-0028l condition not met).

## Provider-by-Provider Findings

### Claude Code (v2.1.226) -- COVERED

Structured output via `--output-format stream-json` provides typed
lifecycle events. `--include-hook-events` adds Stop/UserPromptSubmit/
PreToolUse/PostToolUse hook events to the stream. `--include-partial-messages`
adds streaming progress chunks.

Lifecycle coverage:
- Session start: `--print -p` or `--input-format stream-json`
- Turn boundaries: stream-json events (turn start/end)
- Tool use: PostToolUse hook events in stream
- Rate limit: NOT exposed structurally (terminal scrape only)
- Graceful shutdown: SIGINT, `--max-turns`

Rate-limit detection remains a terminal-adapter concern. All other
lifecycle events are available structurally.

### Gemini CLI (v0.53.1) -- COVERED

`--output-format stream-json` mirrors Claude Code's structured output
shape. Gemini CLI also has a hooks system (`gemini hooks`).

Lifecycle coverage:
- Session start: `-p` prompt mode
- Turn boundaries: stream-json events
- Progress/tool use: partial coverage via stream-json
- Rate limit: NOT exposed structurally
- Graceful shutdown: SIGINT

### Codex CLI (v0.141.0) -- COVERED

`codex exec --json` prints events to stdout as JSONL, providing typed
lifecycle events. Also supports hooks (with `--dangerously-bypass-hook-trust`
for automation).

Lifecycle coverage:
- Session start: `exec` subcommand
- Turn boundaries: JSONL events
- Tool use: JSONL tool events
- Rate limit: NOT exposed structurally
- Graceful shutdown: SIGINT

### Ollama (v0.20.5) -- NOT COVERED

Ollama is a raw LLM model server, not a coding agent CLI. It exposes a
REST API (`/api/chat`, `/api/generate`) with streaming JSON responses,
but provides none of the agent-level features:

- No file editing or tool use
- No hooks system
- No structured lifecycle events beyond HTTP stream chunks
- No session/turn concept
- Rate limiting exposed only as HTTP 429

Ollama serves as a model BACKEND for other agent CLIs (e.g., Codex's
`--oss --local-provider ollama`), not as a standalone agent. When used
through another CLI, that CLI's structured events apply.

## Written Branch Decision

Condition: `OpenCode coverage < 3 of 4 providers for core lifecycle events`

Result: **3 of 4 covered** (Claude Code, Gemini CLI, Codex CLI).
Ollama is not an agent CLI and is not expected to provide agent lifecycle events.

Decision: **Written branch does NOT fire.** `TerminalAdapter` remains the
fallback for rate-limit detection and providers without structured output.
`OpenCodeAdapter` is the primary protocol path.

No Phase 1+4 re-estimate required (RR-0028l).

## Remaining Gap: Rate Limits

All three covered providers lack structured rate-limit detection. Rate-limit
patterns are provider-specific and terminal-only:
- Claude Code: regex matching rate-limit menu patterns
- Gemini CLI: regex matching quota/retry patterns
- Codex CLI: regex matching rate-limit patterns

The terminal adapter's rate-limit detection remains load-bearing for all
providers. This is consistent with the plan's event coverage table
(Invariant 5): `RateLimited` shows `--` for both OpenCode and hooks columns.

## herdr Agent Detection

herdr (v0.8.0) already has agent-detection profiles for claude, codex, and
gemini in `~/.local/state/herdr/agent-detection/remote/`. These detect
terminal UI patterns (working/blocked/permission states) via regex rules.
The detection profiles confirm the terminal adapter's role as the rate-limit
and TUI-state fallback.
