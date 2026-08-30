//! Auto-relay: when sessions reply to Telegram-routed messages, send replies back to Telegram.
//!
//! # Design
//!
//! For each session with an active Telegram mapping:
//! 1. Peek its output (last 300 lines)
//! 2. Find the LAST `[from Telegram @...]` message in the pane
//! 3. Extract all NEW text after that message (using line number checkpoint)
//! 4. Send it back to Telegram with HTML formatting
//! 5. Update checkpoint so we don't send dupes
//!
//! Runs every 30 seconds. No state in-process; everything checkpointed in DB.
//! Failures are logged but never propagated — a Telegram send error doesn't kill the relay job
//! or block other sessions. The user can always resend from the session via curl if needed.

use super::registry;
use crate::api::sessions_legacy::{is_chrome_line, strip_ansi};
use crate::api::session_verbs::tmux_capture;
use crate::api::AppState;
use crate::db::telegram as tg_db;
use std::time::Duration;

const JOB: &str = "telegram_relay";

pub async fn run(state: AppState) {
    loop {
        registry::tick(JOB);
        if let Err(e) = relay_cycle(&state).await {
            tracing::warn!("telegram_relay: scan error: {e}");
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

async fn relay_cycle(state: &AppState) -> Result<(), String> {
    // Get all Telegram mappings (which sessions are linked to which chats)
    let mappings = {
        let conn = state.store.read().map_err(|e| e.to_string())?;
        tg_db::list(&conn).map_err(|e| e.to_string())?
    };

    for mapping in &mappings {
        if let Err(e) = check_and_relay(state, mapping).await {
            // Log but continue — don't let one session's error block others
            tracing::debug!(
                "telegram_relay: chat {} (watching '{}'): {}",
                mapping.chat_id, mapping.routed_session(), e
            );
        }
    }

    Ok(())
}

/// Pure text half of the relay: given a session's raw pane capture (as
/// `tmux capture-pane -e` returns it, ANSI intact), finds the LAST
/// `[from Telegram @...]` marker and returns the cleaned reply text after it,
/// plus the total line count (the checkpoint `check_and_relay` records via
/// `mark_relayed`). Kept separate from `check_and_relay` — which also does
/// real tmux I/O and a real HTTP send — specifically so this half is
/// unit-testable against a captured incident shape, not just against the
/// live pane.
///
/// `tmux capture-pane -e` preserves raw ANSI escape sequences — needed
/// elsewhere for color-aware rendering, but Telegram has no ANSI renderer, so
/// left in place they show up as literal garbage ("[38;5;231m..."). Strip
/// ONCE, up front, so both the marker search and the reply extraction work on
/// the same clean text. The lines after the marker are then filtered through
/// `is_chrome_line` — the same predicate `preview_of` uses to build session
/// previews — to drop TUI chrome: the bottom status bar ("bypass
/// permissions... shift+tab to cycle"), box-drawing dividers, and other
/// terminal furniture a live pane always has BELOW the actual reply text.
///
/// Found live 2026-08-30: raw escape codes AND leaked TUI chrome both reached
/// Telegram verbatim before either strip existed.
fn extract_reply(raw_output: &str) -> Option<(String, i64)> {
    let output = strip_ansi(raw_output);
    let lines: Vec<&str> = output.lines().collect();

    // Find the LAST `[from Telegram @...]` line
    let tg_idx = lines.iter().rposition(|line| line.contains("[from Telegram @"))?;

    // Extract new output lines AFTER the Telegram message, dropping chrome.
    let reply_lines: Vec<&str> =
        lines[(tg_idx + 1)..].iter().map(|s| s.trim()).filter(|s| !is_chrome_line(s)).collect();

    if reply_lines.is_empty() {
        return None;
    }
    Some((reply_lines.join("\n"), lines.len() as i64))
}

async fn check_and_relay(state: &AppState, mapping: &tg_db::TelegramMapping) -> Result<(), String> {
    // Watch wherever the chat's LAST inbound message actually routed to — the
    // `/link`'d default, or an `@lane` target (migration 0040). Reading
    // `mapping.session` directly here is the exact bug found 2026-08-30:
    // `@frontstage status` runs frontstage correctly, but a relay pinned to
    // the static default never looks at frontstage's pane, so the reply is
    // never seen and Telegram gets no feedback at all.
    let watch_session = mapping.routed_session();
    let raw_output = tmux_capture(watch_session, 300).await;

    let Some((reply_text, last_line)) = extract_reply(&raw_output) else {
        // Either no Telegram message in this session's pane yet, or no new
        // (non-chrome) output since it — session probably still working.
        return Ok(());
    };

    // Convert markdown to Telegram HTML
    let html = markdown_to_html(&reply_text);

    // Try sending; fall back to plain text on format error
    match send_reply_to_telegram(mapping.chat_id, &html, Some("HTML")).await {
        Ok(_) => {
            // Record success (line number of last relayed line)
            let _ = state
                .store
                .write_async({
                    let chat_id = mapping.chat_id;
                    move |conn| {
                        tg_db::mark_relayed(conn, chat_id, last_line)?;
                        Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                    }
                })
                .await;
            tracing::info!(
                "telegram_relay: relayed {} bytes from '{}' to chat {}",
                html.len(),
                watch_session,
                mapping.chat_id
            );
            Ok(())
        }
        Err(e) => {
            // Record error (for observability), but don't fail the job
            let _ = state
                .store
                .write_async({
                    let chat_id = mapping.chat_id;
                    let err_msg = e.clone();
                    move |conn| {
                        let _ = tg_db::mark_relay_error(conn, chat_id, &err_msg);
                        Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                    }
                })
                .await;

            // On HTML format error, try plain text
            if e.contains("parse entities") {
                tracing::warn!("telegram_relay: HTML format failed, retrying as plain text for chat {}", mapping.chat_id);
                let plain = strip_html(&html);
                match send_reply_to_telegram(mapping.chat_id, &plain, None).await {
                    Ok(_) => {
                        // Update checkpoint even though we downgraded to plain text
                        let _ = state
                            .store
                            .write_async({
                                let chat_id = mapping.chat_id;
                                move |conn| {
                                    tg_db::mark_relayed(conn, chat_id, last_line)?;
                                    Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
                                }
                            })
                            .await;
                        return Ok(());
                    }
                    Err(e2) => return Err(format!("plain text also failed: {e2}")),
                }
            }

            Err(e)
        }
    }
}

/// Send text to a Telegram chat. Reuses the existing send_message logic from telegram_poll.rs
/// to ensure consistent behavior (including parse_mode support and fallback).
async fn send_reply_to_telegram(
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<(), String> {
    // Use the same send_message from telegram_poll
    crate::runtime_jobs::telegram_poll::send_message(&bot_token()?, chat_id, text, parse_mode).await
}

fn bot_token() -> Result<String, String> {
    std::env::var("TELEGRAM_BOT_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "TELEGRAM_BOT_TOKEN not set".to_string())
}

/// Simple markdown → Telegram HTML converter (subset: bold, italic, code).
/// Same logic as the relay hook's converter for consistency.
fn markdown_to_html(md: &str) -> String {
    let mut result = String::with_capacity(md.len() * 2);
    let mut in_code = false;

    let mut chars = md.chars().peekable();
    while let Some(ch) = chars.next() {
        // Handle code blocks (backticks) with entity encoding inside
        if ch == '`' {
            if in_code {
                result.push_str("</code>");
                in_code = false;
            } else {
                result.push_str("<code>");
                in_code = true;
            }
            continue;
        }

        if in_code {
            // Inside code: escape HTML entities
            match ch {
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                _ => result.push(ch),
            }
            continue;
        }

        // Outside code blocks: handle markdown formatting
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    // **bold**
                    chars.next();
                    result.push_str("<b>");
                    let mut found_close = false;
                    // clippy's `for c in chars.by_ref()` suggestion does NOT
                    // compile here: the for-loop holds a mutable borrow of
                    // `chars` for the whole body, which conflicts with the
                    // `chars.peek()` call below (E0499). `peek()` inside the
                    // loop is what makes `while let` the only form that works.
                    #[allow(clippy::while_let_on_iterator)]
                    while let Some(c) = chars.next() {
                        if c == '*' && chars.peek() == Some(&'*') {
                            chars.next();
                            result.push_str("</b>");
                            found_close = true;
                            break;
                        } else {
                            result.push(c);
                        }
                    }
                    if !found_close {
                        result.push_str("</b>");
                    }
                } else {
                    // *italic*
                    result.push_str("<i>");
                    let mut found_close = false;
                    for c in chars.by_ref() {
                        if c == '*' {
                            result.push_str("</i>");
                            found_close = true;
                            break;
                        } else {
                            result.push(c);
                        }
                    }
                    if !found_close {
                        result.push_str("</i>");
                    }
                }
            }
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            _ => result.push(ch),
        }
    }

    // Ensure no unclosed code tags
    if in_code {
        result.push_str("</code>");
    }

    result
}

fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    registry::spawn_loop(JOB, Some(Duration::from_secs(30)), run(state))
}

#[cfg(test)]
mod tests {
    use super::extract_reply;

    /// Reproduces the incident's own artifact (ethos rule 7: test against
    /// what actually happened, not a convenient shape) — a capture built from
    /// the live `@frontstage status` failure reported 2026-08-30: raw SGR
    /// escape sequences interleaved with the reply text, then TUI chrome
    /// (bypass-permissions status bar, box-drawing dividers, a session-link
    /// card) filling the rest of the 300-line capture below it, exactly as
    /// `tmux capture-pane -e` returns a live Claude Code pane.
    #[test]
    fn strips_ansi_and_chrome_but_keeps_the_real_reply() {
        let raw = "\
> @frontstage status
[from Telegram @ivomarino]: @frontstage status
\u{1b}[38;5;231m\u{1b}[49m\u{1b}[39m\u{1b}[1mFrame Phase 1 Status Update:\u{1b}[0m
\u{1b}[32m✅\u{1b}[0m \u{1b}[1mCode Review Complete\u{1b}[0m — All Markdown files reviewed
\u{1b}[1mKey Finding:\u{1b}[0m Phase 1 is deployment + compliance, NOT development.
- Node.js Express app is 100% complete and ready to deploy

────────────────────────────────────────────
────────────────────────────────────────────
────────────────────────────────────────────

 \u{1b}[39m  \u{1b}[38;5;211m⏵⏵ bypass permissions on \u{1b}[38;5;246m(shift+tab to cycle) · ← for agents \u{1b}[39m
\u{1b}[38;5;114m ]8;id=fpvemu;https://claude.ai/code/session_01WvxNFGUdXVcHPpuuGHTvEJ?from=cli\u{1b}\\/rc \u{1b}[39m]8;;\u{1b}\\
Claude Code
A shared Claude Code session on claude.ai/code";

        let (reply, last_line) = extract_reply(raw).expect("a reply was present");

        // The garbage that was reaching Telegram verbatim must be gone.
        assert!(!reply.contains('\u{1b}'), "raw ANSI escape byte leaked: {reply:?}");
        assert!(!reply.contains("[38;5;"), "an escape sequence's tail leaked as literal text: {reply:?}");
        assert!(!reply.contains("bypass permissions"), "status-bar chrome leaked: {reply:?}");
        assert!(!reply.contains("shift+tab"), "status-bar chrome leaked: {reply:?}");
        assert!(!reply.contains("────"), "a box-drawing divider row leaked: {reply:?}");
        assert!(!reply.contains("claude.ai/code/session_"), "the session-link card leaked: {reply:?}");
        assert!(
            !reply.lines().any(|l| l.trim() == "Claude Code"),
            "the session-link card's standalone heading leaked: {reply:?}"
        );

        // The actual reply content must survive, readable.
        assert!(reply.contains("Frame Phase 1 Status Update:"), "{reply:?}");
        assert!(reply.contains("Code Review Complete"), "{reply:?}");
        assert!(reply.contains("Node.js Express app is 100% complete"), "{reply:?}");

        assert_eq!(last_line, raw.lines().count() as i64);
    }

    #[test]
    fn no_marker_in_pane_means_nothing_to_relay() {
        assert!(extract_reply("some unrelated pane output\nwith no telegram marker at all").is_none());
    }

    #[test]
    fn marker_with_no_new_output_yet_means_nothing_to_relay() {
        // The session is still working: the marker is the LAST real content,
        // trailing chrome does not count as "a reply".
        let raw = "[from Telegram @ivomarino]: @frontstage status\n\
                   \u{1b}[39m  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← for agents \u{1b}[39m";
        assert!(extract_reply(raw).is_none());
    }

    #[test]
    fn only_the_last_marker_counts_when_a_chat_has_multiple_rounds() {
        let raw = "[from Telegram @ivomarino]: first question\n\
                   first answer, already relayed\n\
                   [from Telegram @ivomarino]: second question\n\
                   second answer, this is the new one";
        let (reply, _) = extract_reply(raw).expect("a reply was present");
        assert_eq!(reply, "second answer, this is the new one");
        assert!(!reply.contains("first answer"), "{reply:?}");
    }
}
