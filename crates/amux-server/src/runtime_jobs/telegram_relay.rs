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
use crate::api::AppState;
use crate::api::session_verbs::tmux_capture;
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

async fn check_and_relay(state: &AppState, mapping: &tg_db::TelegramMapping) -> Result<(), String> {
    // Watch wherever the chat's LAST inbound message actually routed to — the
    // `/link`'d default, or an `@lane` target (migration 0040). Reading
    // `mapping.session` directly here is the exact bug found 2026-08-30:
    // `@frontstage status` runs frontstage correctly, but a relay pinned to
    // the static default never looks at frontstage's pane, so the reply is
    // never seen and Telegram gets no feedback at all.
    let watch_session = mapping.routed_session();
    // Capture the session's current terminal output (last 300 lines)
    let output = tmux_capture(watch_session, 300).await;
    let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();

    // Find the LAST `[from Telegram @...]` line
    let mut tg_line_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate().rev() {
        if line.contains("[from Telegram @") {
            tg_line_idx = Some(i);
            break;
        }
    }

    let Some(tg_idx) = tg_line_idx else {
        // No Telegram message in this session's pane, nothing to relay
        return Ok(());
    };

    // Extract new output lines AFTER the Telegram message
    let reply_lines: Vec<String> = lines[(tg_idx + 1)..]
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if reply_lines.is_empty() {
        // No new output yet, session probably still working
        return Ok(());
    }

    // Convert reply to text and send back
    let reply_text = reply_lines.join("\n");

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
                    let last_line = lines.len() as i64;
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
                                let last_line = lines.len() as i64;
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
