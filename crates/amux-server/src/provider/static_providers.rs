//! Minimal adapters for gemini, codex, and ollama (RR-0043).
//!
//! "Minimal" is a statement about USAGE, not a placeholder: none of the three
//! exposes a usage/quota API amux can read on this host today
//! (docs/provider-coverage.csv: rate limits are terminal-scrape-only for all
//! of them), so `usage()` returns [`ProviderUsage::unknown`] — zero windows,
//! headline [`UsageConfidence::Unknown`]. Inventing a number here is exactly
//! what Invariant 20 forbids, and routing (RR-0044) treats Unknown as
//! "never exhausted" rather than guessing.
//!
//! Capability claims below cite the OpenCode spike
//! (docs/opencode-spike-results.md, RR-0028e) — they are measured, not
//! aspirational.

use amux_core::provider::{ProviderCapabilities, ProviderId, ProviderUsage};
use async_trait::async_trait;

use super::{PromptMode, ProviderAdapter};

// ---------------------------------------------------------------------------
// Gemini CLI
// ---------------------------------------------------------------------------

/// Gemini CLI (spike: v0.53.1). `--output-format stream-json` mirrors Claude
/// Code's structured shape; a hooks system exists (`gemini hooks`).
pub struct GeminiAdapter;

#[async_trait]
impl ProviderAdapter for GeminiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new("gemini")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            hot_model_switch: false,
            // No readable usage surface (terminal scrape only, per spike).
            reports_usage: false,
            // stream-json per the spike coverage matrix.
            structured_events: true,
            // `gemini hooks` (docs/provider-coverage.csv).
            hooks: true,
        }
    }

    async fn usage(&self) -> ProviderUsage {
        // No usage API to read; unknown is the only honest answer.
        ProviderUsage::unknown(self.id())
    }

    async fn models(&self) -> Vec<String> {
        // The CLI's own selectable tiers; no listing endpoint to query.
        vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()]
    }

    fn build_command(&self, prompt_mode: PromptMode) -> Vec<String> {
        match prompt_mode {
            PromptMode::Interactive => vec!["gemini".into()],
            // Non-interactive when stdin is a pipe; structured events via
            // stream-json (spike: mirrors Claude Code's shape). The prompt
            // arrives over stdin — the protocol's job, not argv's.
            PromptMode::HeadlessStructured => vec![
                "gemini".into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Codex CLI
// ---------------------------------------------------------------------------

/// Codex CLI (spike: v0.141.0). `codex exec --json` emits typed JSONL
/// lifecycle events.
pub struct CodexAdapter;

#[async_trait]
impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new("codex")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            hot_model_switch: false,
            // Usage-limit messages are terminal text only (spike).
            reports_usage: false,
            // JSONL via `codex exec --json` (spike coverage matrix).
            structured_events: true,
            // Hooks exist (spike: `--dangerously-bypass-hook-trust` for
            // automation).
            hooks: true,
        }
    }

    async fn usage(&self) -> ProviderUsage {
        ProviderUsage::unknown(self.id())
    }

    async fn models(&self) -> Vec<String> {
        // No enumerable model surface from the CLI; empty is honest — the
        // configured model rides in WorkerConfig, not here.
        Vec::new()
    }

    fn build_command(&self, prompt_mode: PromptMode) -> Vec<String> {
        match prompt_mode {
            PromptMode::Interactive => vec!["codex".into()],
            // Typed JSONL events on stdout (spike).
            PromptMode::HeadlessStructured => {
                vec!["codex".into(), "exec".into(), "--json".into()]
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama
// ---------------------------------------------------------------------------

/// Ollama (spike: v0.20.5) — a local model SERVER, not an agent CLI: no
/// hooks, no structured lifecycle events from the `ollama` binary, no usage
/// accounting (locally unlimited != a number; still Unknown, zero windows).
pub struct OllamaAdapter {
    /// `ollama run <model>` requires a model in argv. This default is
    /// CONFIGURATION, not knowledge — callers should override it from
    /// `WorkerConfig.model`; the default only keeps a bare adapter runnable.
    pub default_model: String,
}

impl Default for OllamaAdapter {
    fn default() -> Self {
        Self {
            default_model: "llama3".into(),
        }
    }
}

impl OllamaAdapter {
    pub fn with_model(model: impl Into<String>) -> Self {
        Self {
            default_model: model.into(),
        }
    }
}

#[async_trait]
impl ProviderAdapter for OllamaAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new("ollama")
    }

    fn capabilities(&self) -> ProviderCapabilities {
        // All false — the conservative default IS the measured truth here
        // (spike: "raw LLM server only; no coding agent features").
        ProviderCapabilities::default()
    }

    async fn usage(&self) -> ProviderUsage {
        // "Effectively unlimited" is not a number amux may invent; the
        // absence of windows says exactly what is known: nothing.
        ProviderUsage::unknown(self.id())
    }

    async fn models(&self) -> Vec<String> {
        // The one place a live listing exists: the local `ollama list`
        // subprocess. Binary missing / daemon down / timeout -> empty vec,
        // the honest answer on a host without ollama.
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("ollama").arg("list").output(),
        )
        .await;
        match out {
            Ok(Ok(o)) if o.status.success() => {
                parse_ollama_list(&String::from_utf8_lossy(&o.stdout))
            }
            _ => Vec::new(),
        }
    }

    fn build_command(&self, prompt_mode: PromptMode) -> Vec<String> {
        // Both modes launch `ollama run <model>`: interactively it is a REPL;
        // headless (piped stdin) it answers and exits. There is no structured
        // argv mode — ollama's structured surface is its REST API, which is a
        // protocol concern, not a spawn command (spike: not an agent CLI).
        let _ = prompt_mode;
        vec!["ollama".into(), "run".into(), self.default_model.clone()]
    }
}

/// Parse `ollama list` output: a header line, then one model per line with
/// the name as the first whitespace-separated column, e.g.
/// `llama3:latest    365c0bd3c000    4.7 GB    2 weeks ago`.
fn parse_ollama_list(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .skip(1) // "NAME  ID  SIZE  MODIFIED"
        .filter_map(|line| line.split_whitespace().next())
        .map(|name| name.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use amux_core::provider::UsageConfidence;

    #[tokio::test]
    async fn static_usage_is_unknown_with_zero_windows() {
        for adapter in [
            &GeminiAdapter as &dyn ProviderAdapter,
            &CodexAdapter,
            &OllamaAdapter::default(),
        ] {
            let usage = adapter.usage().await;
            assert_eq!(usage.provider, adapter.id());
            assert!(
                usage.windows.is_empty(),
                "{}: minimal adapter must report zero windows",
                adapter.id()
            );
            assert_eq!(usage.confidence(), UsageConfidence::Unknown);
        }
    }

    #[test]
    fn capability_claims_match_the_spike() {
        assert!(GeminiAdapter.capabilities().structured_events);
        assert!(!GeminiAdapter.capabilities().reports_usage);
        assert!(CodexAdapter.capabilities().structured_events);
        assert!(!CodexAdapter.capabilities().reports_usage);
        let ollama = OllamaAdapter::default().capabilities();
        assert!(!ollama.structured_events);
        assert!(!ollama.hooks);
        assert!(!ollama.reports_usage);
        assert!(!ollama.hot_model_switch);
    }

    #[test]
    fn ollama_interactive_is_run_model() {
        let a = OllamaAdapter::with_model("qwen3:8b");
        assert_eq!(
            a.build_command(PromptMode::Interactive),
            vec!["ollama", "run", "qwen3:8b"]
        );
        assert_eq!(
            a.build_command(PromptMode::HeadlessStructured),
            vec!["ollama", "run", "qwen3:8b"]
        );
    }

    #[test]
    fn codex_headless_is_exec_json() {
        assert_eq!(
            CodexAdapter.build_command(PromptMode::HeadlessStructured),
            vec!["codex", "exec", "--json"]
        );
    }

    #[test]
    fn parses_ollama_list_output() {
        let fixture = "NAME               ID              SIZE      MODIFIED\n\
                       llama3:latest      365c0bd3c000    4.7 GB    2 weeks ago\n\
                       qwen3:8b           500a1f067a9f    5.2 GB    3 days ago\n";
        assert_eq!(
            parse_ollama_list(fixture),
            vec!["llama3:latest", "qwen3:8b"]
        );
        assert!(parse_ollama_list("").is_empty());
        assert!(parse_ollama_list("NAME  ID  SIZE  MODIFIED\n").is_empty());
    }
}
