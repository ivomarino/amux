//! Claude Code adapter (RR-0043).
//!
//! Usage comes from the Claude subscription OAuth usage endpoint — a port of
//! the Python probe (`_claude_oauth_token` / `_fetch_claude_usage` in
//! amux-server.py, ~line 3160): read the OAuth token from the macOS keychain
//! (fallback `~/.claude/.credentials.json`), then GET
//! `https://api.anthropic.com/api/oauth/usage`. STRICTLY read-only: one GET,
//! no token refresh, no writes anywhere.
//!
//! Invariant 20 discipline: every failure — no token, expired token, spawn
//! error, HTTP error, unparseable body — collapses to
//! [`ProviderUsage::unknown`]. The adapter records what Anthropic reported
//! (utilization percentages) in the unit it was reported in; it never
//! converts a percentage into invented token counts.

use std::path::PathBuf;
use std::time::Duration;

use amux_core::provider::{
    ProviderCapabilities, ProviderId, ProviderUsage, UsageConfidence, UsageProvenance,
    UsageWindow, UsageWindowKind,
};
use async_trait::async_trait;

use super::{PromptMode, ProviderAdapter};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Keychain service name Claude Code stores its credentials under.
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// One budget for every external step (keychain subprocess, HTTP call).
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct ClaudeAdapter;

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self
    }

    fn provider_id() -> ProviderId {
        ProviderId::new("claude-code")
    }
}

#[async_trait]
impl ProviderAdapter for ClaudeAdapter {
    fn id(&self) -> ProviderId {
        Self::provider_id()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            // `/model` switches models mid-session without a restart.
            hot_model_switch: true,
            // The OAuth usage endpoint below (UsageProvenance::Api).
            reports_usage: true,
            // stream-json + --include-hook-events (docs/provider-coverage.csv).
            structured_events: true,
            // Stop/UserPromptSubmit/PreToolUse/PostToolUse lifecycle hooks.
            hooks: true,
        }
    }

    async fn usage(&self) -> ProviderUsage {
        let unknown = ProviderUsage::unknown(self.id());
        let Some(token) = oauth_token().await else {
            return unknown;
        };
        // Expired token: the probe must not guess whether Anthropic would
        // still answer; the Python probe reports unavailable here too.
        if token.expires_at_ms > 0
            && chrono::Utc::now().timestamp_millis() > token.expires_at_ms
        {
            return unknown;
        }
        let Ok(client) = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() else {
            return unknown;
        };
        let resp = client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {}", token.access_token))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("anthropic-version", "2023-06-01")
            .send()
            .await;
        let Ok(resp) = resp else { return unknown };
        if !resp.status().is_success() {
            return unknown;
        }
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            return unknown;
        };
        ProviderUsage::new(self.id(), map_usage_response(&body))
    }

    async fn models(&self) -> Vec<String> {
        // Static tier aliases, deliberately: the Claude Code CLI resolves
        // these to concrete model ids itself (`--model opus`), and the
        // subscription OAuth surface exposes no model-listing endpoint — so a
        // live listing would require an API that does not exist for this auth
        // mode. Static is the honest, sufficient answer.
        vec!["opus".into(), "sonnet".into(), "haiku".into()]
    }

    fn build_command(&self, prompt_mode: PromptMode) -> Vec<String> {
        match prompt_mode {
            // The fleet-standard interactive launch (backend::SessionSpec's
            // own doc example).
            PromptMode::Interactive => vec![
                "claude".into(),
                "--dangerously-skip-permissions".into(),
            ],
            // Headless with structured lifecycle events, per the OpenCode
            // spike (docs/opencode-spike-results.md): stream-json in and out
            // (prompts arrive over stdin), --verbose is REQUIRED by the CLI
            // for stream-json output in print mode.
            PromptMode::HeadlessStructured => vec![
                "claude".into(),
                "--print".into(),
                "--input-format".into(),
                "stream-json".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
                "--dangerously-skip-permissions".into(),
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth token (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OauthToken {
    access_token: String,
    /// Unix ms; 0 = the credential did not say.
    expires_at_ms: i64,
}

/// Both credential stores hold the same JSON shape:
/// `{"claudeAiOauth": {"accessToken": "...", "expiresAt": 1234567890123}}`.
fn parse_credentials_json(raw: &str) -> Option<OauthToken> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    let oauth = v.get("claudeAiOauth")?;
    let access_token = oauth.get("accessToken")?.as_str()?.to_string();
    if access_token.is_empty() {
        return None;
    }
    let expires_at_ms = oauth
        .get("expiresAt")
        .and_then(|e| e.as_i64())
        .unwrap_or(0);
    Some(OauthToken {
        access_token,
        expires_at_ms,
    })
}

/// macOS keychain, the primary store. On hosts without `security` (Linux CI)
/// the spawn fails and we fall through to the file — same presence/absence
/// logic as the Python probe, no platform branch.
async fn keychain_token() -> Option<OauthToken> {
    let out = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::process::Command::new("security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_credentials_json(std::str::from_utf8(&out.stdout).ok()?)
}

fn file_token() -> Option<OauthToken> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home).join(".claude").join(".credentials.json");
    parse_credentials_json(&std::fs::read_to_string(path).ok()?)
}

async fn oauth_token() -> Option<OauthToken> {
    if let Some(t) = keychain_token().await {
        return Some(t);
    }
    file_token()
}

// ---------------------------------------------------------------------------
// Response mapping (pure — this is what the unit tests exercise)
// ---------------------------------------------------------------------------

/// Map the OAuth usage response into normalized windows. Two shapes exist in
/// the wild:
///
/// - primary shape: top-level `five_hour` / `seven_day` / `seven_day_opus`
///   objects, each `{utilization, resets_at}` (live-verified 2026-08-09; an
///   absent/null window like `seven_day_opus` on a non-Opus plan simply maps
///   to no window);
/// - the `limits[]` array (what the Python dashboard's `loadUsage()`
///   consumes): entries of `{kind, percent, resets_at, ...}` where
///   `kind == "session"` (live) / `"worker"` (older) is the 5-hour window and
///   weekly/`weekly_scoped` entries are 7-day windows.
///
/// The live endpoint currently returns BOTH at once, describing the same
/// facts — so `limits[]` is a FALLBACK, consumed only when the primary shape
/// yields nothing; mapping both would record each window twice.
///
/// Anthropic reports UTILIZATION PERCENTAGES, not token counts — so the
/// window records exactly that: `used` = reported percent, `limit` = 100
/// (definitional for a percentage, not invented). `used >= limit` means the
/// window is exhausted; over 100 is legal (over-limit is a real state).
/// Entries whose kind we cannot classify are SKIPPED, never guessed into a
/// window kind they might not be (Invariant 20).
fn map_usage_response(v: &serde_json::Value) -> Vec<UsageWindow> {
    let mut windows = Vec::new();

    for (key, kind) in [
        ("five_hour", UsageWindowKind::Rolling),
        ("seven_day", UsageWindowKind::Weekly),
        ("seven_day_opus", UsageWindowKind::Weekly),
    ] {
        if let Some(obj) = v.get(key) {
            if let Some(w) = window_from_percent(obj.get("utilization"), obj.get("resets_at"), kind)
            {
                windows.push(w);
            }
        }
    }
    if !windows.is_empty() {
        return windows;
    }

    if let Some(limits) = v.get("limits").and_then(|l| l.as_array()) {
        for entry in limits {
            let kind_str = entry.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            let group = entry.get("group").and_then(|g| g.as_str()).unwrap_or("");
            let kind = if kind_str == "session" || kind_str == "worker" {
                UsageWindowKind::Rolling
            } else if kind_str.starts_with("weekly") || group == "weekly" {
                UsageWindowKind::Weekly
            } else {
                continue; // unclassifiable: skip, never guess
            };
            if let Some(w) =
                window_from_percent(entry.get("percent"), entry.get("resets_at"), kind)
            {
                windows.push(w);
            }
        }
    }

    windows
}

/// One window from a reported percent + optional ISO-8601 reset time.
/// No numeric percent -> no window (a window with nothing reported would be
/// `Unknown`, and the aggregate already expresses that with zero windows).
fn window_from_percent(
    percent: Option<&serde_json::Value>,
    resets_at: Option<&serde_json::Value>,
    kind: UsageWindowKind,
) -> Option<UsageWindow> {
    let pct = percent?.as_f64()?;
    if !pct.is_finite() || pct < 0.0 {
        return None;
    }
    let resets_at = resets_at
        .and_then(|r| r.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    Some(UsageWindow {
        window_kind: kind,
        used: Some(pct.round() as u64),
        limit: Some(100),
        resets_at,
        confidence: UsageConfidence::Exact, // the provider reported it directly
        provenance: UsageProvenance::Api,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- response mapping (the network-free contract) ----------------------

    #[test]
    fn maps_documented_probe_shape() {
        let fixture = serde_json::json!({
            "five_hour":      {"utilization": 34.4, "resets_at": "2026-08-09T18:00:00Z"},
            "seven_day":      {"utilization": 71.6, "resets_at": "2026-08-12T00:00:00+00:00"},
            "seven_day_opus": {"utilization": 0,    "resets_at": null},
        });
        let windows = map_usage_response(&fixture);
        assert_eq!(windows.len(), 3);

        assert_eq!(windows[0].window_kind, UsageWindowKind::Rolling);
        assert_eq!(windows[0].used, Some(34));
        assert_eq!(windows[0].limit, Some(100));
        assert_eq!(
            windows[0].resets_at.unwrap().to_rfc3339(),
            "2026-08-09T18:00:00+00:00"
        );

        assert_eq!(windows[1].window_kind, UsageWindowKind::Weekly);
        assert_eq!(windows[1].used, Some(72)); // rounded, not truncated
        assert_eq!(windows[2].window_kind, UsageWindowKind::Weekly);
        assert_eq!(windows[2].used, Some(0)); // a REPORTED zero, kept as such
        assert_eq!(windows[2].resets_at, None);

        for w in &windows {
            assert_eq!(w.confidence, UsageConfidence::Exact);
            assert_eq!(w.provenance, UsageProvenance::Api);
        }
    }

    #[test]
    fn maps_live_limits_array_shape_as_fallback() {
        // The limits[] shape alone (no top-level windows): session
        // (kind="session" live / "worker" older), weekly-all, and per-model
        // weekly_scoped entries.
        let fixture = serde_json::json!({
            "limits": [
                {"kind": "session", "percent": 12, "resets_at": "2026-08-09T20:00:00Z"},
                {"kind": "worker", "percent": 9, "resets_at": "2026-08-09T20:00:00Z"},
                {"kind": "weekly_all", "group": "weekly", "percent": 55,
                 "resets_at": "2026-08-13T07:00:00Z"},
                {"kind": "weekly_scoped", "percent": 103,
                 "scope": {"model": {"display_name": "Opus"}},
                 "resets_at": "2026-08-13T07:00:00Z"},
                {"kind": "mystery_future_kind", "percent": 5},
                {"kind": "weekly_all", "group": "weekly"}
            ]
        });
        let windows = map_usage_response(&fixture);
        // mystery kind skipped (never guessed); percent-less entry skipped
        // (nothing reported -> no window, not a fabricated one).
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].window_kind, UsageWindowKind::Rolling);
        assert_eq!(windows[0].used, Some(12));
        assert_eq!(windows[1].window_kind, UsageWindowKind::Rolling);
        assert_eq!(windows[1].used, Some(9));
        assert_eq!(windows[2].window_kind, UsageWindowKind::Weekly);
        assert_eq!(windows[2].used, Some(55));
        // Over-limit stays visible: 103/100, remaining saturates to 0.
        assert_eq!(windows[3].used, Some(103));
        assert_eq!(windows[3].remaining(), Some(0));
        assert_eq!(windows[3].overage(), Some(3));
    }

    #[test]
    fn both_shapes_present_maps_primary_only() {
        // The live endpoint (verified 2026-08-09) carries BOTH shapes
        // describing the same facts; mapping both would record each window
        // twice, and a doubled window is a small lie about what was reported.
        let fixture = serde_json::json!({
            "five_hour": {"utilization": 58.0, "resets_at": "2026-08-09T19:00:00+00:00"},
            "seven_day": {"utilization": 12.0, "resets_at": "2026-08-16T01:00:00+00:00"},
            "seven_day_opus": null, // absent window on this plan: no window, not a zero
            "limits": [
                {"kind": "session", "percent": 58, "resets_at": "2026-08-09T19:00:00Z"},
                {"kind": "weekly_all", "percent": 12, "resets_at": "2026-08-16T01:00:00Z"},
                {"kind": "weekly_scoped", "percent": 20, "resets_at": "2026-08-16T01:00:00Z"}
            ]
        });
        let windows = map_usage_response(&fixture);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_kind, UsageWindowKind::Rolling);
        assert_eq!(windows[0].used, Some(58));
        assert_eq!(windows[1].window_kind, UsageWindowKind::Weekly);
        assert_eq!(windows[1].used, Some(12));
    }

    #[test]
    fn unparseable_or_empty_bodies_yield_zero_windows() {
        for fixture in [
            serde_json::json!({}),
            serde_json::json!({"available": false, "reason": "no token"}),
            serde_json::json!({"five_hour": {"resets_at": "2026-08-09T18:00:00Z"}}),
            serde_json::json!({"five_hour": {"utilization": "lots"}}),
            serde_json::json!({"five_hour": {"utilization": -3}}),
            serde_json::json!({"limits": "not-an-array"}),
            serde_json::json!([1, 2, 3]),
        ] {
            let windows = map_usage_response(&fixture);
            assert!(
                windows.is_empty(),
                "expected no windows for {fixture}, got {windows:?}"
            );
            // Zero windows -> headline Unknown: the honest aggregate.
            let usage = ProviderUsage::new(ClaudeAdapter::provider_id(), windows);
            assert_eq!(usage.confidence(), UsageConfidence::Unknown);
        }
    }

    #[test]
    fn bad_reset_timestamps_drop_the_timestamp_not_the_window() {
        let fixture = serde_json::json!({
            "five_hour": {"utilization": 50, "resets_at": "not-a-date"},
        });
        let windows = map_usage_response(&fixture);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used, Some(50));
        assert_eq!(windows[0].resets_at, None);
    }

    // -- credential parsing ------------------------------------------------

    #[test]
    fn parses_credentials_json_shape() {
        let raw = r#"{"claudeAiOauth": {"accessToken": "sk-ant-oat-xyz",
                       "expiresAt": 1791234567000}}"#;
        let tok = parse_credentials_json(raw).unwrap();
        assert_eq!(tok.access_token, "sk-ant-oat-xyz");
        assert_eq!(tok.expires_at_ms, 1791234567000);

        // Missing expiresAt is legal (0 = did not say), missing/empty token
        // is not a credential at all.
        let tok = parse_credentials_json(r#"{"claudeAiOauth":{"accessToken":"t"}}"#).unwrap();
        assert_eq!(tok.expires_at_ms, 0);
        assert!(parse_credentials_json(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credentials_json(r#"{"somethingElse":{}}"#).is_none());
        assert!(parse_credentials_json("not json").is_none());
    }

    // -- adapter surface ---------------------------------------------------

    #[test]
    fn capabilities_are_the_full_set() {
        let caps = ClaudeAdapter::new().capabilities();
        assert!(caps.hooks);
        assert!(caps.structured_events);
        assert!(caps.hot_model_switch);
        assert!(caps.reports_usage);
    }

    #[test]
    fn commands_are_well_formed() {
        let a = ClaudeAdapter::new();
        let interactive = a.build_command(PromptMode::Interactive);
        assert_eq!(interactive[0], "claude");
        let headless = a.build_command(PromptMode::HeadlessStructured);
        assert_eq!(headless[0], "claude");
        // stream-json output in print mode requires --verbose, or the CLI
        // rejects the invocation — a session that dies at spawn.
        assert!(headless.contains(&"--verbose".to_string()));
        assert!(headless.contains(&"stream-json".to_string()));
    }

    // -- live probe (network + keychain) -----------------------------------

    /// Real read-only probe against api.anthropic.com using this host's
    /// credentials. #[ignore] because CI has no keychain and no token; run
    /// manually with: cargo test -p amux-server provider -- --ignored
    #[tokio::test]
    #[ignore]
    async fn live_usage_probe_is_honest() {
        let usage = ClaudeAdapter::new().usage().await;
        assert_eq!(usage.provider, ClaudeAdapter::provider_id());
        // Whatever happened — token found or not, HTTP ok or not — the
        // result is either real reported windows or a clean unknown; never
        // numbers under Unknown confidence.
        for w in &usage.windows {
            assert_eq!(w.confidence, UsageConfidence::Exact);
            assert_eq!(w.provenance, UsageProvenance::Api);
            assert!(w.used.is_some() && w.limit == Some(100));
        }
        // Full conformance, network included, belongs here and only here.
        super::super::conformance(&ClaudeAdapter::new()).await;
    }
}
