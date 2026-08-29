//! Telegram bot long-poll loop.
//!
//! Telegram gives two ways to receive updates: a public webhook (needs a
//! reachable HTTPS endpoint — this box is a Docker container behind a
//! Proxmox NAT with no public IP, so that's out) or `getUpdates` long-polling
//! (works from anywhere with outbound HTTPS, which this box already has for
//! every other connector). MVP takes the polling path; a webhook mode can be
//! added later behind whatever tunnel/reverse-proxy setup exists then — the
//! `telegram_poll_state` checkpoint and the mapping table underneath don't
//! change either way.
//!
//! # Message flow
//!
//! - `/link <session>` from a chat that has never linked: validate the
//!   session exists (`session_verbs::all_lane_names`), store the mapping,
//!   confirm. Re-linking (chat already mapped) just repoints it — no
//!   separate unlink-then-link dance for the common "I meant a different
//!   session" case.
//! - Any other text from a LINKED chat: delivered into the mapped session
//!   via `session_verbs::send_text`, stamped with the sender so the session
//!   can tell a Telegram message from anything else arriving in its pane
//!   (same reasoning as the `[amux-origin: ...]` stamp on cross-session
//!   sends — attribution travels with the text, not out-of-band).
//! - Any other text from an UNLINKED chat: a one-line nudge back to
//!   `/link <session>`, never silently dropped (ethos rule 3: the honest
//!   refusal beats the quiet nothing).
//!
//! # Outbound (session -> Telegram)
//!
//! `POST /api/telegram/send` (api/telegram.rs) is the outbound path for now —
//! a session or a hook calls it explicitly. There is no automatic "every
//! session message also goes to Telegram" wiring yet; that's a notification-
//! routing decision (which events, which chats) that belongs to whoever is
//! using this, not a default this job should assume (ethos rule 8).

use super::registry;
use crate::api::session_verbs;
use crate::api::AppState;
use crate::db::telegram as tg_db;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const JOB: &str = registry::ids::TELEGRAM_POLL;

/// Telegram's own recommended long-poll timeout. The HTTP client timeout
/// below must exceed this or every poll looks like a network failure.
const POLL_TIMEOUT_SECS: u64 = 25;

fn bot_token() -> Option<String> {
    std::env::var("TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.trim().is_empty())
}

fn api_base(token: &str) -> String {
    format!("https://api.telegram.org/bot{token}")
}

// ---------------------------------------------------------------------------
// Last-report state for `GET /api/telegram/status` — same shape as the other
// jobs' debug surfaces (registry.rs's module doc: read the job's own report,
// never a second copy of the same fact).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub last_poll_at: Option<f64>,
    pub last_error: Option<String>,
    pub messages_routed: u64,
    pub messages_unlinked: u64,
}

static REPORT: OnceLock<Mutex<Report>> = OnceLock::new();

fn report_slot() -> &'static Mutex<Report> {
    REPORT.get_or_init(|| Mutex::new(Report::default()))
}

pub fn last_report() -> Report {
    report_slot().lock().expect("telegram report lock poisoned").clone()
}

fn record_poll(err: Option<String>) {
    let mut r = report_slot().lock().expect("telegram report lock poisoned");
    r.last_poll_at = Some(registry::unix_now());
    r.last_error = err;
}

fn record_routed() {
    report_slot().lock().expect("telegram report lock poisoned").messages_routed += 1;
}

fn record_unlinked() {
    report_slot().lock().expect("telegram report lock poisoned").messages_unlinked += 1;
}

/// The loop. Never returns — matches every other `spawn_loop` job (gcal-sync,
/// event-processors, ...). Errors are caught and logged PER TICK, not
/// propagated, so one bad response from Telegram's API doesn't kill the loop
/// for the rest of the process lifetime.
pub async fn run(state: AppState) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(POLL_TIMEOUT_SECS + 10))
        .build()
        .expect("reqwest client build");

    let mut warned_no_token = false;
    loop {
        registry::tick(JOB);
        let Some(token) = bot_token() else {
            if !warned_no_token {
                tracing::info!(
                    "telegram_poll: TELEGRAM_BOT_TOKEN not set — idling until a token is pasted \
                     into the connector and the server restarts (connectors are env-loaded at boot)"
                );
                warned_no_token = true;
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        };
        warned_no_token = false;

        match poll_once(&client, &token, &state).await {
            Ok(()) => record_poll(None),
            Err(e) => {
                tracing::warn!("telegram_poll: {e}");
                record_poll(Some(e));
                // Back off past Telegram's own long-poll window so a
                // persistent failure (bad token, network down) doesn't spin
                // hot — same shape as gcal-sync's per-account catch-and-continue.
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }
}

async fn poll_once(client: &reqwest::Client, token: &str, state: &AppState) -> Result<(), String> {
    let offset = {
        let conn = state.store.read().map_err(|e| e.to_string())?;
        tg_db::last_update_id(&conn).map_err(|e| e.to_string())?
    };
    let url = format!(
        "{}/getUpdates?offset={}&timeout={}",
        api_base(token),
        offset + 1,
        POLL_TIMEOUT_SECS
    );
    let resp = client.get(&url).send().await.map_err(|e| format!("getUpdates request: {e}"))?;
    let status = resp.status();
    let body: Value =
        resp.json().await.map_err(|e| format!("getUpdates body (status {status}): {e}"))?;
    if !status.is_success() || body.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("getUpdates rejected (status {status}): {body}"));
    }
    let updates = body.get("result").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut max_update_id = offset;
    for update in &updates {
        if let Some(id) = update.get("update_id").and_then(Value::as_i64) {
            max_update_id = max_update_id.max(id);
        }
        handle_update(state, update).await;
    }
    if max_update_id > offset {
        state
            .store
            .write_async(move |conn| {
                tg_db::set_last_update_id(conn, max_update_id)?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn handle_update(state: &AppState, update: &Value) {
    let Some(msg) = update.get("message") else { return };
    let Some(chat_id) = msg.get("chat").and_then(|c| c.get("id")).and_then(Value::as_i64) else {
        return;
    };
    let text = msg.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let username = msg
        .get("from")
        .and_then(|f| f.get("username"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if text.is_empty() {
        return;
    }

    if let Some(session) = text.strip_prefix("/link ").map(str::trim) {
        link_chat(state, chat_id, session, username.as_deref()).await;
        return;
    }

    let mapping = {
        let Ok(conn) = state.store.read() else { return };
        tg_db::by_chat(&conn, chat_id).ok().flatten()
    };
    let Some(mapping) = mapping else {
        record_unlinked();
        send_reply(chat_id, "Not linked to any amux session yet. Send `/link <session-name>` first.").await;
        return;
    };

    let _ = state
        .store
        .write_async(move |conn| {
            tg_db::touch_last_message(conn, chat_id)?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await;

    let who = username.as_deref().unwrap_or("telegram");
    let stamped = format!("[from Telegram @{who}]: {text}");
    let (ok, msg) =
        session_verbs::send_text(state, &mapping.session, &stamped, true, session_verbs::SendOrigin::Automation)
            .await;
    if ok {
        record_routed();
    } else {
        tracing::warn!(
            "telegram_poll: delivery to session '{}' failed for chat {}: {}",
            mapping.session,
            chat_id,
            msg
        );
        send_reply(chat_id, &format!("Couldn't deliver to '{}': {}", mapping.session, msg)).await;
    }
}

async fn link_chat(state: &AppState, chat_id: i64, session: &str, username: Option<&str>) {
    if session.is_empty() {
        send_reply(chat_id, "Usage: /link <session-name>").await;
        return;
    }
    let known = session_verbs::all_lane_names();
    if !known.iter().any(|n| n == session) {
        send_reply(
            chat_id,
            &format!("No such session '{session}'. Known sessions: {}", known.join(", ")),
        )
        .await;
        return;
    }
    let session_owned = session.to_string();
    let username_owned = username.map(str::to_string);
    let stored = state
        .store
        .write_async(move |conn| {
            tg_db::upsert(conn, chat_id, &session_owned, username_owned.as_deref())?;
            Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
        })
        .await
        .map_err(|e| e.to_string());
    match stored {
        Ok(_) => send_reply(chat_id, &format!("Linked. Messages here now go to '{session}'.")).await,
        Err(e) => {
            tracing::warn!("telegram_poll: link write failed for chat {chat_id}: {e}");
            send_reply(chat_id, "Internal error linking — try again.").await;
        }
    }
}

/// Fire-and-forget outbound send, used for the bot's own replies (link
/// confirmation, unlinked nudge, delivery-failure notice). Errors are logged,
/// not propagated — a failed reply must never take down the poll loop that's
/// still holding real Telegram updates to process.
async fn send_reply(chat_id: i64, text: &str) {
    let Some(token) = bot_token() else { return };
    // Plain text, no formatting — these are short fixed status strings
    // ("Linked.", the unlinked nudge), never worth the entity-parsing risk.
    if let Err(e) = send_message(&token, chat_id, text, None).await {
        tracing::warn!("telegram_poll: reply to chat {chat_id} failed: {e}");
    }
}

/// Strip HTML tags for the plain-text fallback below — not a sanitizer (this
/// text is going TO Telegram, not being embedded in a page we render), just
/// good enough to turn `<b>x</b>` into `x` instead of a user reading raw
/// angle brackets when entity parsing rejected the formatted version. Also
/// unwinds the handful of entities the sender's HTML converter produces
/// (`&amp;`/`&lt;`/`&gt;`/`&quot;`/`&#39;`) — no crate for this, the input is
/// our own converter's output, not arbitrary untrusted HTML.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// `POST {base}/sendMessage`. `pub(crate)` so `api::telegram`'s
/// `POST /api/telegram/send` can reuse the exact same call rather than a
/// second HTTP-shape spelling of it (D6).
///
/// `parse_mode: Some("HTML")` lets a caller send Telegram's HTML-subset
/// markup (core.telegram.org/bots/api#formatting-options — bold/italic/
/// strikethrough/code/pre/links/blockquote; no native headings/lists/
/// tables). Telegram REJECTS the whole message on a single malformed
/// entity (an unclosed tag, a bad href) — since the caller's HTML came from
/// a regex-based converter over arbitrary markdown, not a real parser, that
/// is a real failure mode, not a hypothetical one. On that specific
/// rejection this retries ONCE as plain text (tags stripped) rather than
/// dropping the message: a reply with visible `<b>` tags, or no reply at
/// all, is worse than one that lost its formatting but still arrived
/// (ethos rule 3 — the honest degradation, not a silent loss).
pub(crate) async fn send_message(
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let mut body = serde_json::json!({ "chat_id": chat_id, "text": text });
    if let Some(mode) = parse_mode {
        body["parse_mode"] = serde_json::Value::String(mode.to_string());
    }
    let resp = client
        .post(format!("{}/sendMessage", api_base(token)))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("sendMessage request: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let err_body = resp.text().await.unwrap_or_default();
    // Only retry when formatting was actually in play and Telegram's error
    // shape says it choked on the entities, not on rate limits/auth/network.
    if parse_mode.is_some() && status.as_u16() == 400 && err_body.to_lowercase().contains("parse entities") {
        tracing::warn!(
            "telegram_poll: sendMessage rejected formatted text for chat {chat_id}, retrying as plain: {err_body}"
        );
        let plain = strip_html_tags(text);
        let retry = client
            .post(format!("{}/sendMessage", api_base(token)))
            .json(&serde_json::json!({ "chat_id": chat_id, "text": plain }))
            .send()
            .await
            .map_err(|e| format!("sendMessage plain-text retry: {e}"))?;
        let retry_status = retry.status();
        if retry_status.is_success() {
            return Ok(());
        }
        let retry_body = retry.text().await.unwrap_or_default();
        return Err(format!(
            "sendMessage rejected formatted (status {status}): {err_body}; plain-text retry also rejected (status {retry_status}): {retry_body}"
        ));
    }
    Err(format!("sendMessage rejected (status {status}): {err_body}"))
}

pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    registry::spawn_loop(JOB, Some(Duration::from_secs(POLL_TIMEOUT_SECS)), run(state))
}
