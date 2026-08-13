//! `GET /api/sessions/<n>/simple` — a plain-English ("Basic English") summary of
//! what a worker just did, for the peek **Simple** tab (Ethan 2026-08-13,
//! AMUX-3056).
//!
//! Generated from the worker's LAST assistant message via the SAME
//! fastest/cheapest helper as `/api/lookup` (`lookup::helper_answer` — resident
//! local model, else the CLI; `AMUX_HELPER_MODEL` still wins). Cached per
//! session and only regenerated when the transcript advances OR the standing
//! prompt changes (keyed by a hash of prompt+message), so opening the tab is
//! instant and a brief idle does not re-spend a model call.
//!
//! SUMMARIZE, not verbatim (confirmed with Ethan): the last message is usually
//! the turn's wrap-up but can be a question or terse, so the model is asked to
//! state, in simple English, what the worker just DID — robust either way.
//!
//! Config (font/size/styling) is client-side; the STANDING PROMPT is passed in
//! via `?prompt=` (the client owns the global-default + per-worker override and
//! sends the resolved one), falling back to `DEFAULT_PROMPT` here.

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const DEFAULT_PROMPT: &str = "You are explaining to a non-technical person what an AI \
coding assistant just finished doing. Rewrite the message below as a short summary in \
SIMPLE ENGLISH: short sentences, common everyday words (Basic English), no jargon, no \
code, no markdown. Say plainly what was done and why it matters. 2-4 sentences.";

/// Newest ASSISTANT text from the session's transcript tail. Reads the tail
/// directly (not `iter_jsonl_tail`) and walks lines backwards, so it does not
/// depend on that helper's record ordering — the same tail-then-reverse shape
/// `transcript_evidence` uses. A long lane's transcript is hundreds of MB; only
/// the last records describe NOW.
fn last_assistant_text(name: &str) -> Option<String> {
    let path = crate::api::session_verbs::session_jsonl_path(name)?;
    let f = std::fs::File::open(&path).ok()?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    const TAIL: u64 = 512_000;
    let mut rdr = std::io::BufReader::new(f);
    if len > TAIL {
        use std::io::Seek;
        let _ = rdr.seek(std::io::SeekFrom::Start(len - TAIL));
    }
    use std::io::Read;
    let mut buf = String::new();
    let _ = rdr.read_to_string(&mut buf);
    for line in buf.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let msg = &v["message"];
        // A record is `{"type":"assistant","message":{"role":"assistant",...}}`
        // — accept either the message role or the record type.
        let role = msg["role"].as_str().or_else(|| v["type"].as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }
        let text = extract_text(&msg["content"]);
        if !text.trim().is_empty() {
            // Cap: the summary only needs the wrap-up, and the helper prompt has
            // its own budget.
            return Some(text.trim().chars().take(8000).collect());
        }
    }
    None
}

/// Claude Code `content` is either a bare string or an array of blocks; keep the
/// text blocks (tool_use/tool_result blocks carry no prose worth summarizing).
fn extract_text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    let Some(arr) = content.as_array() else { return String::new() };
    let mut out = String::new();
    for b in arr {
        if b["type"].as_str() == Some("text") {
            if let Some(t) = b["text"].as_str() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    out
}

/// (source_key, summary, via) — a session's cached summary and how it was made.
type CacheEntry = (u64, String, String);

/// session -> its cached summary. In-memory: a summary is cheap to regenerate
/// and must not survive a transcript it no longer describes.
fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static C: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_key(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The tab endpoint body, callable from the session-verb dispatcher (per-session
/// routes go through one catch-all handler, so this is NOT an axum extractor).
/// `prompt` is the client-resolved standing prompt (global default + per-worker
/// override); `None` uses `DEFAULT_PROMPT`. `refresh` forces a regenerate past
/// the cache.
pub async fn simple_response(name: &str, prompt: Option<&str>, refresh: bool) -> Response {
    let Some(last) = last_assistant_text(name) else {
        // Honest empty state, not an error: a fresh lane simply has no assistant
        // turn yet. The tab shows this verbatim.
        return Json(json!({
            "summary": Value::Null,
            "cached": false,
            "reason": "no assistant message in this worker's transcript yet",
        }))
        .into_response();
    };
    let prompt = prompt
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(DEFAULT_PROMPT);
    // Key on prompt AND message: a new turn OR a changed standing prompt both
    // invalidate. \u{0} cannot appear in either, so it is an unambiguous joiner.
    let key = hash_key(&format!("{prompt}\u{0}{last}"));
    if !refresh {
        if let Ok(c) = cache().lock() {
            if let Some((k, summary, via)) = c.get(name) {
                if *k == key {
                    return Json(json!({
                        "summary": summary,
                        "via": via,
                        "source_key": key.to_string(),
                        "cached": true,
                    }))
                    .into_response();
                }
            }
        }
    }
    let full = format!("{prompt}\n\n---\n{last}");
    match crate::api::lookup::helper_answer(&full).await {
        Ok((via, summary)) => {
            if let Ok(mut c) = cache().lock() {
                c.insert(name.to_string(), (key, summary.clone(), via.clone()));
            }
            Json(json!({
                "summary": summary,
                "via": via,
                "source_key": key.to_string(),
                "cached": false,
            }))
            .into_response()
        }
        Err((code, msg)) => {
            (code, Json(json!({"summary": Value::Null, "error": msg}))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_text_handles_string_and_block_array() {
        assert_eq!(extract_text(&json!("hi there")), "hi there");
        let blocks = json!([
            {"type":"text","text":"first"},
            {"type":"tool_use","name":"Bash"},
            {"type":"text","text":"second"}
        ]);
        assert_eq!(extract_text(&blocks), "first\nsecond");
        // A tool-only message yields no prose.
        assert_eq!(extract_text(&json!([{"type":"tool_result","content":"x"}])), "");
    }

    #[test]
    fn the_key_changes_with_both_prompt_and_message() {
        let a = hash_key(&format!("{}\u{0}{}", "P1", "M1"));
        let b = hash_key(&format!("{}\u{0}{}", "P2", "M1"));
        let c = hash_key(&format!("{}\u{0}{}", "P1", "M2"));
        assert_ne!(a, b, "changing the standing prompt must invalidate");
        assert_ne!(a, c, "a new turn (new message) must invalidate");
    }
}
