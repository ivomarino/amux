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
//! The model is not pinned to a weak local one. The DEFAULT is the cheapest
//! Claude model (`DEFAULT_HELPER_MODEL = "haiku"`, a CLI alias that tracks the
//! latest haiku), overridable live from the dashboard settings (`helper_model`
//! pref) and by `AMUX_HELPER_MODEL` (D3, highest priority). An override that
//! names an ollama model (`name:tag`) runs locally. This improves as the models
//! do, and no longer depends on a resident ollama model being available.

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


/// The local ollama endpoint, if one is running. `AMUX_OLLAMA_URL` overrides.
fn ollama_url() -> String {
    std::env::var("AMUX_OLLAMA_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434".into())
}

/// How long ollama should keep the weights resident. This is the whole
/// difference between a 3s lookup and a 19s one; the default (5m) evicts the
/// model between uses on a machine doing anything else.
fn ollama_keep_alive() -> String {
    std::env::var("AMUX_OLLAMA_KEEP_ALIVE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "30m".into())
}

/// The cheapest, fastest Claude model, used as the DEFAULT for every quick meta
/// task (Look Up, the Orchestrate router, the plain-English worker summary). A
/// bare alias, not a dated id, so it tracks the latest haiku as the CLI improves
/// (D3: never pin a weak model). Ethan (2026-08-17): these tasks used to fall to
/// the smallest resident ollama model, which was often qwen and "isn't always
/// available"; the default is now Claude.
const DEFAULT_HELPER_MODEL: &str = "haiku";

/// An ollama model id is `name:tag` (e.g. `qwen3.8:27b`); Claude/codex aliases
/// and ids never contain a colon. That is how the resolved helper model is
/// routed to the local ollama runner vs the helper CLI.
fn is_ollama_model(m: &str) -> bool {
    m.contains(':')
}

/// The live, no-restart override for the meta-task model, set from the dashboard
/// settings and stored in the `prefs` table (`helper_model`). Read with a
/// short-lived read-only connection so the one shared seam (`helper_answer`)
/// stays stateless; any error (no DB, no table, no row, empty) returns None and
/// the default applies. It never breaks a lookup.
fn helper_model_pref() -> Option<String> {
    let db = std::env::var("AMUX_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| crate::config::amux_home().join("amux.db"));
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let v: String = conn
        .query_row("SELECT value FROM prefs WHERE key='helper_model'", [], |r| {
            r.get(0)
        })
        .ok()?;
    let v = v.trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// The effective meta-task model, highest priority first:
///   1. `AMUX_HELPER_MODEL` env — the D3 knob, so server.env deployments win;
///   2. the `helper_model` pref — the live dashboard override (any configured
///      model across the harness: a Claude alias/id, or an ollama `name:tag`);
///   3. `DEFAULT_HELPER_MODEL` — the cheap Claude default.
fn resolve_helper_model() -> String {
    if let Ok(m) = std::env::var("AMUX_HELPER_MODEL") {
        let m = m.trim();
        if !m.is_empty() {
            return m.to_string();
        }
    }
    helper_model_pref().unwrap_or_else(|| DEFAULT_HELPER_MODEL.to_string())
}

/// Ask a resident local model BY NAME. Returns the answer or None to fall
/// through to the CLI — never an error, because "ollama not running / model not
/// pulled" is not a failure of the lookup, it is a machine without that model.
async fn try_local_model_named(prompt: &str, model: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(LOOKUP_TIMEOUT_S))
        .build()
        .ok()?;
    let v: Value = client
        .post(format!("{}/api/generate", ollama_url()))
        .json(&json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "keep_alive": ollama_keep_alive(),
        }))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let answer = v["response"].as_str()?.trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

/// Fastest, cheapest one-shot answer for a fully-formed prompt: a resident LOCAL
/// model when no `AMUX_HELPER_MODEL` is pinned, else the helper CLI (D3 — the one
/// knob still wins). This is the ONE place the "fastest cheapest model" seam
/// lives, shared by `/api/lookup` (explain-selection) and
/// `/api/sessions/<n>/simple` (the plain-English worker summary) so it cannot
/// drift into two spellings that must be kept in step forever (D6). Returns
/// `(via, text)` — `via` is `ollama:<model>` or the CLI name — or an HTTP
/// status + message. The caller supplies the WHOLE prompt (this does not wrap
/// it), so each caller keeps its own instruction.
pub(crate) async fn helper_answer(prompt: &str) -> Result<(String, String), (StatusCode, String)> {
    let model = resolve_helper_model();
    if is_ollama_model(&model) {
        if let Some(answer) = try_local_model_named(prompt, &model).await {
            return Ok((format!("ollama:{model}"), answer));
        }
        // The chosen local model is unavailable — fall through to the cheap
        // Claude default rather than the CLI's own (heavier) default.
    }
    let cli = std::env::var("AMUX_HELPER_CLI").unwrap_or_else(|_| "claude".into());
    // Use the resolved model as the CLI model, unless it was an ollama id that
    // just failed above, in which case the cheap Claude default answers.
    let cli_model = if is_ollama_model(&model) {
        DEFAULT_HELPER_MODEL.to_string()
    } else {
        model
    };
    let mut cmd = tokio::process::Command::new(&cli);
    cmd.arg("--print").arg(prompt);
    if !cli_model.is_empty() {
        cmd.arg("--model").arg(&cli_model);
    }
    cmd.stdin(std::process::Stdio::null());
    match tokio::time::timeout(std::time::Duration::from_secs(LOOKUP_TIMEOUT_S), cmd.output()).await {
        Err(_) => Err((
            StatusCode::GATEWAY_TIMEOUT,
            format!("{cli} did not answer within {LOOKUP_TIMEOUT_S}s"),
        )),
        Ok(Err(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not run {cli}: {e}"),
        )),
        Ok(Ok(out)) => {
            // Non-empty stdout wins even on a non-zero exit (the CLI prints its
            // answer then sometimes exits non-zero); only an EMPTY answer is a
            // failure — the whole lookup incident was a zero-byte body.
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if stdout.is_empty() {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    if err.is_empty() {
                        format!("{cli} exited without output")
                    } else {
                        err.chars().take(400).collect()
                    },
                ));
            }
            Ok((format!("{cli}:{cli_model}"), stdout))
        }
    }
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

    // Fastest, cheapest, on this machine — a resident LOCAL model first, else
    // the CLI. That whole decision (and the measured 3s-ollama vs 7-24s-CLI
    // rationale, and the `AMUX_HELPER_MODEL` D3 override) now lives in ONE place,
    // `helper_answer`, shared with the Simple worker-summary endpoint (D6). The
    // client reads only `text`.
    match helper_answer(&prompt_for(&text)).await {
        Ok((via, answer)) => (StatusCode::OK, Json(json!({"text": answer, "via": via}))),
        Err((code, msg)) => (code, Json(json!({"error": msg}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_models_are_colon_tagged_and_the_default_is_cheap_claude() {
        // Routing: colon-tagged ids go to ollama, Claude aliases/ids to the CLI.
        assert!(is_ollama_model("qwen3.8:27b"));
        assert!(is_ollama_model("llama3:8b"));
        assert!(!is_ollama_model("haiku"));
        assert!(!is_ollama_model("claude-haiku-4-5"));
        assert!(!is_ollama_model("sonnet"));
        // The default is the cheap Claude model, not a local model.
        assert_eq!(DEFAULT_HELPER_MODEL, "haiku");
    }

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
