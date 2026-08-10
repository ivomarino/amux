//! `POST /api/lookup` — "explain this selection" from the peek view.
//!
//! Ethan selected text in a pane, hit Look up, and got:
//!
//! ```text
//! Lookup failed: Failed to execute 'json' on 'Response': Unexpected end of JSON input
//! ```
//!
//! The endpoint was never ported from Python. Nothing was mounted at
//! `/api/lookup`, so the GET-only SPA catch-all answered the POST with 405 and
//! a ZERO-BYTE body, and the client's unconditional `r.json()` threw a parser
//! error that names neither the path nor the status.
//!
//! It WAS logged, and correctly: `GET /api/logs/analyze` had it as
//! `405 POST /api/lookup, count=10` with the verdict already written —
//! "no route exists at this path — the 405 is the GET-only SPA catch-all
//! answering a non-GET; treat as an unknown path (404-class)". The
//! instrumentation did its job; nobody was reading it. (Ethos rule 4: the tag
//! existed, the reader never opened the store.)
//!
//! # Deviations from the Python original, stated rather than hidden
//!
//! Python rode the peeked session's PROVIDER (gemini/ollama/codex) and fell
//! back to claude. This port always uses the configured helper CLI. That is a
//! real reduction for a gemini-only or ollama-only setup, and it is called out
//! here rather than discovered later — porting the provider-riding needs the
//! per-provider argv table, which does not exist in the Rust tree yet.
//!
//! The model is NOT pinned. `AMUX_HELPER_MODEL` selects it and an unset value
//! means "whatever the CLI defaults to", so this improves when the CLI does
//! (D3: a hardcoded weak-model helper is a bet that cannot improve).

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use super::AppState;

const MAX_TEXT: usize = 2000;
const LOOKUP_TIMEOUT_S: u64 = 45;

fn prompt_for(text: &str) -> String {
    format!(
        "Briefly explain what this means or refers to in 2-4 sentences. Be concise and \
         direct. If it's a technical term, code, error, or concept, explain it. If it's a \
         name, identify it.\n\n{text}"
    )
}

pub async fn lookup(
    State(_state): State<AppState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let text: String = body
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_TEXT)
        .collect();
    if text.is_empty() {
        // A JSON body even on refusal. The whole incident was a zero-byte
        // response meeting an unconditional r.json(), so every exit from this
        // handler carries a parseable body.
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing text"})),
        );
    }

    let cli = std::env::var("AMUX_HELPER_CLI").unwrap_or_else(|_| "claude".into());
    let mut cmd = tokio::process::Command::new(&cli);
    cmd.arg("--print").arg(prompt_for(&text));
    // Unset means "the CLI's own default", which is what lets this improve
    // without a code change.
    if let Ok(m) = std::env::var("AMUX_HELPER_MODEL") {
        if !m.trim().is_empty() {
            cmd.arg("--model").arg(m.trim());
        }
    }
    cmd.stdin(std::process::Stdio::null());

    let run = tokio::time::timeout(
        std::time::Duration::from_secs(LOOKUP_TIMEOUT_S),
        cmd.output(),
    )
    .await;

    match run {
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"error": format!("{cli} did not answer within {LOOKUP_TIMEOUT_S}s")})),
        ),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            // Name the binary. "No such file or directory" alone sent the
            // reader looking at the server instead of at a missing CLI.
            Json(json!({"error": format!("could not run {cli}: {e}")})),
        ),
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !out.status.success() && stdout.is_empty() {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "error": if err.is_empty() {
                            format!("{cli} exited without output")
                        } else {
                            err.chars().take(400).collect::<String>()
                        }
                    })),
                );
            }
            if stdout.is_empty() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("{cli} returned an empty answer")})),
                );
            }
            (StatusCode::OK, Json(json!({"text": stdout, "provider": cli})))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_carries_the_selection_verbatim() {
        let p = prompt_for("SIGPIPE");
        assert!(p.contains("SIGPIPE"));
        assert!(p.contains("2-4 sentences"), "the brevity instruction is the point");
    }

    /// The selection comes from a terminal pane, so it can be enormous. The cap
    /// is applied to CHARS, not bytes — slicing bytes would panic on a
    /// multi-byte boundary, which a pane full of box-drawing characters
    /// reliably produces.
    #[test]
    fn an_enormous_multibyte_selection_is_capped_without_panicking() {
        let huge: String = "├─╢".repeat(5000);
        let capped: String = huge.trim().chars().take(MAX_TEXT).collect();
        assert_eq!(capped.chars().count(), MAX_TEXT);
    }
}
