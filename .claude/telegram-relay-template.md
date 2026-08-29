# Telegram Auto-Reply Hook — Amux Reference

**Status**: The relay hook is built into amux and works. Universal worker setup is future work.

## Current State (Proven)

**Amux** has auto-reply built-in:
- When someone sends `message` to amux, it routes to the amux session
- Amux's Claude responds
- The Stop hook fires and auto-sends the reply back to Telegram
- Formatting (bold, italic, code, etc.) preserved via HTML
- ✅ Tested and verified live

## For Other Workers (Interim)

Until the relay hook setup works universally, **any worker can explicitly call the Telegram API** to reply:

```bash
# From your session: reply to a Telegram message routed to you
curl -sk -X POST -H 'Content-Type: application/json' \
  -d '{"session":"your-session-name","text":"Your reply"}' \
  $AMUX_URL/api/telegram/send
```

This gives you **direct control** over when and what gets sent to Telegram (ethos rule 8).

## How the Hook Works (Reference)

1. Telegram message routed via `@session-name` → `[from Telegram @user]: ...` appears in pane
2. Session's Claude responds normally
3. Stop event fires (turn ends)
4. Relay hook detects `[from Telegram @...]` attribution
5. Hook calls `POST /api/telegram/send` with formatted reply
6. Message appears on Telegram with HTML formatting

## Code Reference

- `.claude/telegram-relay.py` — the hook script (generic, session-agnostic)
- `.claude/settings.json` — amux's Stop hook configuration
- Markdown→Telegram HTML converter inside the hook

## Future

The relay hook pattern should work for any worker once Claude Code's hook system supports dynamic registration or session-specific configurations. Until then, the explicit REST API approach is the reliable path.
