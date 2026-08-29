# Telegram Auto-Reply Hook — Setup for Any Worker

Any worker running inside amux can auto-reply to Telegram messages routed to it via the `@session-name` mention system.

## Setup (copy-paste into your worker's `.claude/settings.json`)

1. Copy `.claude/telegram-relay.py` from amux into your worker's `.claude/` directory
2. Add this to your `.claude/settings.json` under `hooks.Stop`:

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "python3 $CLAUDE_PROJECT_DIR/.claude/telegram-relay.py",
            "timeout": 15
          }
        ]
      }
    ]
  }
}
```

## How it works

1. Someone sends `@your-session-name your message` to the Telegram bot
2. Message routes into your session's Claude pane with attribution: `[from Telegram @user]: ...`
3. Your Claude responds normally in the session
4. When the Stop event fires (turn ends), the relay hook detects the Telegram-stamped message
5. Hook auto-sends your reply back to Telegram with HTML formatting (bold, italic, code, etc.)

## Features

- ✅ Idempotent: Won't double-send if Stop fires multiple times for the same turn
- ✅ Fail-open: Never blocks your session; errors logged to stderr
- ✅ HTML formatting: Converts markdown to Telegram HTML with fallback to plain text
- ✅ Works with any session: Uses `$AMUX_SESSION` environment variable

## Example

```
[Telegram sends: "@frontstage what's the status?"]
↓
[Message appears in frontstage's pane with attribution]
[from Telegram @user]: what's the status?
↓
[frontstage's Claude responds naturally]
[your detailed status report here...]
↓
[Stop event fires, relay hook detects Telegram attribution]
↓
[Reply auto-sends back to Telegram with formatting]
```

## Troubleshooting

- Hook runs silently (fail-open design)
- Check `~/.amux/logs/` or your session logs for errors
- If reply doesn't arrive, verify:
  - Telegram message has `[from Telegram @...]` attribution
  - Your reply has actual text content (not empty)
  - Session name matches the chat mapping

## Notes

This is the long-term clean solution per ethos rule 8 (each worker decides when/what to send). Amux gets the hook built-in; other workers opt-in by copying it.
