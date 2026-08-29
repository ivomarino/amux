#!/usr/bin/env python3
"""Stop hook: mirror a session's reply back to Telegram when the turn that
produced it was triggered by an inbound Telegram message.

Why this exists: telegram_poll.rs routes inbound text INTO a session (stamped
`[from Telegram @user]: ...`), but nothing routes a reply back OUT — that's a
deliberate, documented gap (skills/amux.md's Telegram section: "no automatic
... wiring, an explicit call, not a background forwarder"). Explicit meant
"someone has to remember to call POST /api/telegram/send every time," and in
practice that's exactly what got missed the first time this ran live
(2026-08-29): the reply landed in the terminal, never reached the phone, and
looked like silence to the person on the other end. This hook is the fix, at
the layer the miss actually happened — a human forgetting a step during a
conversation, not a server bug — so it runs on EVERY Stop, for every amux
session, not just the one that happened to hit it once (ethos rule 1: an
exemption belongs at the top, not baked into whichever session got patched).

Fires on Claude Code's `Stop` event (this repo's .claude/settings.json), once
per finished turn, for whichever session is running. Fails open on every
uncertainty per this repo's hook convention (session-freshness.sh's header) —
a hook that can block a reply is worse than the silent gap it exists to close.

Mechanics:
  1. Read the transcript, walk backward via parentUuid from the tail to find
     the LAST user message and the assistant text that answered it.
  2. If that user message is not stamped `[from Telegram @...]:`, do nothing
     — most turns are not Telegram-triggered, and this must stay silent for
     all of them.
  3. Idempotency: track the last user-message uuid this hook has already
     relayed for THIS session in ~/.amux/telegram-relay/<session>.uuid, so a
     Stop that fires again with no new Telegram-stamped turn since (e.g. a
     sub-agent's own Stop, or a re-fire) does not resend the same reply.
  4. Convert the assistant's markdown to what Telegram's HTML parse_mode
     supports (bold/italic/strikethrough/code/pre/links/blockquote — no
     native headings/tables/lists, degraded per convert_markdown_to_telegram_html
     below), truncate to Telegram's 4096-char cap, POST to
     /api/telegram/send.
"""
import html
import json
import os
import re
import sys
import urllib.error
import urllib.request

STAMP_RE = re.compile(r"^\[from Telegram @(\S+)\]:\s*")


def read_stdin_json():
    try:
        return json.loads(sys.stdin.read())
    except Exception:
        return {}


def load_transcript(path):
    """uuid -> entry, in file order. Tolerates partial/corrupt trailing lines
    (a Stop hook can fire while the last write is still flushing)."""
    entries = {}
    order = []
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    d = json.loads(line)
                except Exception:
                    continue
                u = d.get("uuid")
                if u:
                    entries[u] = d
                    order.append(u)
    except OSError:
        return {}, []
    return entries, order


def extract_text(message):
    """Concatenate `type: text` content blocks only — skips tool_use/
    tool_result/thinking blocks, which are not what a human on Telegram wants
    to read as the reply."""
    content = message.get("content") if isinstance(message, dict) else None
    if isinstance(content, str):
        return content.strip()
    if not isinstance(content, list):
        return ""
    parts = []
    for block in content:
        if isinstance(block, dict) and block.get("type") == "text":
            t = block.get("text", "")
            if t:
                parts.append(t)
    return "\n".join(parts).strip()


def find_last_turn(entries, order):
    """From the tail, walk parentUuid backward: the most recent assistant
    text (possibly split across several assistant entries interleaved with
    tool calls) that follows the most recent user entry. Returns
    (user_entry, assistant_text) or (None, None) if the chain doesn't
    resolve within a bounded number of hops (fails open, not infinite loop)."""
    if not order:
        return None, None
    cur_uuid = order[-1]
    assistant_texts = []
    hops = 0
    while cur_uuid and hops < 500:
        hops += 1
        entry = entries.get(cur_uuid)
        if entry is None:
            return None, None
        etype = entry.get("type")
        if etype == "assistant":
            t = extract_text(entry.get("message", {}))
            if t:
                assistant_texts.append(t)
        elif etype == "user":
            # Skip synthetic tool-result "user" turns (they carry
            # toolUseResult, not something a person typed) — keep walking
            # past them to find the actual human/Telegram-stamped message.
            if entry.get("toolUseResult") is not None:
                cur_uuid = entry.get("parentUuid")
                continue
            return entry, "\n\n".join(reversed(assistant_texts)).strip()
        cur_uuid = entry.get("parentUuid")
    return None, None


CODE_BLOCK_RE = re.compile(r"```(\w*)\n(.*?)```", re.DOTALL)
INLINE_CODE_RE = re.compile(r"`([^`\n]+)`")
# Bold+italic together (***x***) MUST be matched before the separate bold and
# italic passes below — bold alone on '***x***' strips the outer '**' and
# leaves a stray '*x*' on each side, but worse, running bold then italic
# independently produces CROSSED tags (<b><i>x</b></i>), which is invalid
# HTML and Telegram rejects the entire message for it (confirmed: this is
# exactly what shipped before this comment existed).
BOLD_ITALIC_RE = re.compile(r"\*\*\*(.+?)\*\*\*")
BOLD_RE = re.compile(r"\*\*(.+?)\*\*")
ITALIC_RE = re.compile(r"(?<!\*)\*(?!\*)([^*\n]+?)\*(?!\*)|(?<!_)_(?!_)([^_\n]+?)_(?!_)")
STRIKE_RE = re.compile(r"~~(.+?)~~")
LINK_RE = re.compile(r"\[([^\]]+)\]\((\S+?)\)")
HEADING_RE = re.compile(r"^#{1,6}\s+(.*)$", re.MULTILINE)
HR_RE = re.compile(r"^\s*([-*_]){3,}\s*$", re.MULTILINE)
BLOCKQUOTE_RE = re.compile(r"^>\s?(.*)$", re.MULTILINE)

TELEGRAM_TEXT_LIMIT = 4096
# Applied to the RAW markdown, BEFORE convert_markdown_to_telegram_html —
# never to the HTML output. A cut on the final HTML string can land inside a
# tag ('...<b>bo' with no close) and there is no cheap way to tell truncated
# text from a tag boundary after the fact; cutting the plain source first
# means the converter always sees a complete (if shorter) markdown string and
# its balanced-tag guarantee (BOLD_ITALIC_RE etc.) still holds. The 3500
# headroom below the hard 4096 cap covers the HTML tags' own overhead, which
# a plain-text cut point can't see coming.
SAFE_TRUNCATE_AT = 3500


def convert_markdown_to_telegram_html(md):
    """Best-effort markdown -> Telegram HTML parse_mode, covering what
    Claude's own replies actually use. Telegram has no native heading, list,
    or table rendering (core.telegram.org/bots/api#formatting-options) — each
    is degraded explicitly below rather than left to render as raw
    punctuation:
      - headings         -> bold line (there is no <h1>..<h6> in Telegram's
                             HTML subset)
      - bullet/numbered lists -> left as plain text ("- x" / "1. x" already
                             read fine unrendered; Telegram has no list tag)
      - tables            -> wrapped in <pre> so columns stay visually
                             aligned instead of the pipes rendering as noise
      - horizontal rules  -> a plain divider line (no native <hr>)
    """
    placeholders = []

    def stash(html_fragment):
        placeholders.append(html_fragment)
        return f"\x00{len(placeholders) - 1}\x00"

    # 1. Fenced code blocks first (protect their content from every later
    #    regex — a code sample containing '**' or '_' must not be touched).
    def code_block(m):
        lang = m.group(1)
        body = html.escape(m.group(2).rstrip("\n"))
        cls = f' class="language-{html.escape(lang)}"' if lang else ""
        return stash(f"<pre><code{cls}>{body}</code></pre>")

    md = CODE_BLOCK_RE.sub(code_block, md)

    # 2. Naive markdown table detector: 3+ consecutive lines containing '|'
    #    with a separator row (---|---). No real table tag exists in
    #    Telegram's HTML subset, so render it monospaced instead of leaving
    #    raw pipes for the reader to parse by eye.
    def table_block(m):
        return stash(f"<pre>{html.escape(m.group(0).rstrip())}</pre>")

    # [ \t]*, not \s*: \s matches newlines too, so a greedy \s*$ swallows the
    # blank line AFTER the table into the match (confirmed: 'a|b\n---\n1|2\n'
    # followed by a blank line and unrelated text lost that blank line to
    # this pattern before the [ \t] fix landed).
    table_re = re.compile(
        r"(?:^\|.*\|[ \t]*$\n){2,}(?:^\|.*\|[ \t]*$\n?)*", re.MULTILINE
    )
    md = table_re.sub(table_block, md)

    # 3. Inline code spans.
    md = INLINE_CODE_RE.sub(lambda m: stash(f"<code>{html.escape(m.group(1))}</code>"), md)

    # 4. Links — escape href and text independently before either touches
    #    the shared HTML-escape pass below (they're stashed, so they won't).
    md = LINK_RE.sub(
        lambda m: stash(f'<a href="{html.escape(m.group(2), quote=True)}">{html.escape(m.group(1))}</a>'),
        md,
    )

    # 5. Headings -> bold (stash so the '**' added here isn't re-processed
    #    by the bold pass below).
    md = HEADING_RE.sub(lambda m: stash(f"<b>{html.escape(m.group(1).strip())}</b>"), md)

    # 6. Horizontal rules -> plain divider.
    md = HR_RE.sub(lambda m: stash("──────────"), md)

    # 7. Blockquotes: collapse consecutive '> ' lines into one <blockquote>.
    #    MUST run before the html.escape() pass below — BLOCKQUOTE_RE matches
    #    a leading '>', and escaping first turns every '>' into '&gt;',
    #    which no longer matches (confirmed: this exact bug shipped in the
    #    first version, every blockquote rendered as literal '&gt; text').
    def blockquote_block(m):
        lines = [ln[1:].lstrip() if ln.startswith(">") else ln for ln in m.group(0).splitlines()]
        inner = "\n".join(BLOCKQUOTE_RE.sub(r"\1", ln) for ln in lines)
        return stash(f"<blockquote>{html.escape(inner)}</blockquote>")

    quote_run_re = re.compile(r"(?:^>.*$\n?)+", re.MULTILINE)
    md = quote_run_re.sub(blockquote_block, md)

    # 8. Escape whatever plain text is left BEFORE inserting our own tags,
    #    so a literal '<' or '&' the model wrote can't be mistaken for
    #    markup or break the ones we add next.
    md = html.escape(md)

    # 9. Inline emphasis on the now-escaped text. Bold+italic (***x***) FIRST
    #    — see BOLD_ITALIC_RE's comment for why running bold/italic
    #    separately on triple-asterisk text produces invalid crossed tags.
    md = BOLD_ITALIC_RE.sub(lambda m: f"<b><i>{m.group(1)}</i></b>", md)
    md = BOLD_RE.sub(lambda m: f"<b>{m.group(1)}</b>", md)
    md = STRIKE_RE.sub(lambda m: f"<s>{m.group(1)}</s>", md)
    md = ITALIC_RE.sub(lambda m: f"<i>{m.group(1) or m.group(2)}</i>", md)

    # 10. Restore stashed HTML fragments (code/links/headings/tables/hr/blockquotes).
    def restore(m):
        return placeholders[int(m.group(1))]

    md = re.sub(r"\x00(\d+)\x00", restore, md)

    return md.strip()


def truncate_markdown(text):
    """Cut the RAW markdown, not the converted HTML — see SAFE_TRUNCATE_AT's
    comment. Any markdown token left unpaired by the cut (an opening '**'
    with no matching close, an unterminated ``` fence) just falls through
    convert_markdown_to_telegram_html's regexes as literal text instead of
    becoming a tag — ugly at the very margin, never unbalanced HTML."""
    if len(text) <= SAFE_TRUNCATE_AT:
        return text
    cut = text[:SAFE_TRUNCATE_AT].rstrip()
    return cut + "\n\n… (truncated — full reply is in the amux terminal)"


def state_path(session):
    d = os.path.expanduser("~/.amux/telegram-relay")
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, f"{session}.uuid")


def already_relayed(session, user_uuid):
    p = state_path(session)
    try:
        with open(p) as f:
            return f.read().strip() == user_uuid
    except OSError:
        return False


def mark_relayed(session, user_uuid):
    try:
        with open(state_path(session), "w") as f:
            f.write(user_uuid)
    except OSError:
        pass


def send_to_telegram(base_url, session, html_text):
    payload = json.dumps({"session": session, "text": html_text, "parse_mode": "HTML"}).encode()
    req = urllib.request.Request(
        f"{base_url}/api/telegram/send",
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        import ssl

        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        with urllib.request.urlopen(req, timeout=10, context=ctx) as resp:
            return resp.status, resp.read().decode(errors="replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")
    except Exception as e:
        return None, str(e)


def main():
    hook_input = read_stdin_json()
    transcript_path = hook_input.get("transcript_path")
    if not transcript_path or not os.path.isfile(transcript_path):
        return  # fail open — no transcript, nothing to relay

    session = os.environ.get("AMUX_SESSION", "").strip()
    if not session:
        return  # fail open — not running as a named amux session

    entries, order = load_transcript(transcript_path)
    user_entry, assistant_text = find_last_turn(entries, order)
    if user_entry is None or not assistant_text:
        return

    user_text = extract_text(user_entry.get("message", {}))
    m = STAMP_RE.match(user_text)
    if not m:
        return  # this turn wasn't triggered by Telegram — stay silent

    user_uuid = user_entry.get("uuid", "")
    if user_uuid and already_relayed(session, user_uuid):
        return  # already sent this one back

    base_url = os.environ.get("AMUX_URL", "https://localhost:8824").rstrip("/")
    # Truncate BEFORE converting — see truncate_markdown's docstring.
    html_text = convert_markdown_to_telegram_html(truncate_markdown(assistant_text))
    if not html_text.strip():
        return

    status, body = send_to_telegram(base_url, session, html_text)
    if user_uuid and status and 200 <= status < 300:
        mark_relayed(session, user_uuid)
    elif status is None or status >= 400:
        # Fail open (never block Stop), but leave a trace an amux logs sweep
        # would catch — per CLAUDE.md's two-fix rule, a bug fix must surface
        # in amux logs, and this hook running silently on every failure would
        # be exactly the kind of miss it was written to close.
        sys.stderr.write(f"telegram-relay: send failed status={status} body={body[:200]}\n")


if __name__ == "__main__":
    main()
