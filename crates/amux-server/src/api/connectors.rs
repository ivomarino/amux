//! Connectors (integrations): the provider registry + list/status + the secure
//! credential paste. A connector is NOT a new subsystem — scope, env, MCP and
//! browser are the primitives it composes (docs/design/connectors.md,
//! docs/design/connectors-setup.md). This module owns only what those
//! primitives do not:
//!
//!   1. the PROVIDER REGISTRY ([`REGISTRY`]) — a const table so "add another
//!      connector" is one row plus Ethan supplying its client/key. This is the
//!      trivial-to-add mechanism the design promises; the tab renders whatever
//!      the registry declares, so a new provider needs no new endpoint or UI.
//!   2. `GET /api/connectors` — every provider with its connection STATUS, the
//!      env-var NAMES it needs and whether each is SET (masked, never the
//!      value), the OAuth redirect URI to register, and the human setup note.
//!   3. `POST /api/connectors/{id}/credentials` — Ethan pastes a key/secret in
//!      the tab; it is written to `~/.amux/server.env` (the ONE place credential
//!      VALUES live — never the repo, never the DB, never a log) via
//!      [`crate::api::settings::set_server_env_key`], redacted from logs, and
//!      only ever echoed back masked.
//!
//! SCOPE (global / group / worker) is the existing `connectors` scope
//! capability (scope.rs); the tab drives it through `PUT /api/scope` with
//! `capability=connectors`, so this module never re-implements scoping. That is
//! the whole point of the design: a connector is a scopable capability, and the
//! precedence, write-authorization and audit row come from the scope primitive
//! for free.
//!
//! Single-codebase rule: nothing here branches on cloud vs local. A connector
//! is configured or it is not (env presence), and the gateway injects whatever
//! differs. The redirect URI follows [`crate::config::canonical_port`] so it is
//! correct in the 8824 laptop and the 8822 container without a build flag.

use super::AppState;
use crate::config::{amux_home, canonical_port, parse_env_file};
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

/// Auth model for a provider. OAuth needs a client id/secret plus a browser
/// grant; ApiKey needs one pasted secret. Not every connector is OAuth —
/// Granola is key-only — and the registry says so per row rather than assuming.
#[derive(Clone, Copy)]
enum Auth {
    ApiKey {
        key_env: &'static str,
    },
    OAuth2 {
        client_id_env: &'static str,
        client_secret_env: &'static str,
        /// Path the OAuth client must have registered as a redirect URI, joined
        /// to this server's canonical origin at request time. All Google
        /// connectors share ONE client, so they share one callback path.
        callback_path: &'static str,
        scopes: &'static str,
    },
}

#[derive(Clone, Copy)]
struct Provider {
    id: &'static str,
    label: &'static str,
    category: &'static str,
    auth: Auth,
    /// One-line note about what Ethan must do out-of-band (a plan tier, admin
    /// consent, an API to enable). Shown in the tab; empty for none.
    setup_note: &'static str,
    /// A docs/console URL the tab links for setup; empty for none.
    docs: &'static str,
}

/// The registry. Add a connector by adding a row here (plus Ethan supplying its
/// client id/secret or API key). The five in the design plus Slack.
const REGISTRY: &[Provider] = &[
    Provider {
        id: "granola",
        label: "Granola",
        category: "Notes / transcripts",
        auth: Auth::ApiKey { key_env: "GRANOLA_API_KEY" },
        setup_note: "Business or Enterprise plan required to mint a key: Granola desktop -> Settings -> Connectors -> API keys (grn_...). Key-only, no OAuth.",
        docs: "https://public-api.granola.ai",
    },
    Provider {
        id: "google-gmail",
        label: "Gmail",
        category: "Google",
        auth: Auth::OAuth2 {
            client_id_env: "GOOGLE_OAUTH_CLIENT_ID",
            client_secret_env: "GOOGLE_OAUTH_CLIENT_SECRET",
            callback_path: "/api/connectors/google/callback",
            scopes: "https://www.googleapis.com/auth/gmail.modify",
        },
        setup_note: "Enable the Gmail API on the GCP project. All Google connectors share one OAuth client; register the redirect URI below on it.",
        docs: "https://console.cloud.google.com/apis/library/gmail.googleapis.com",
    },
    Provider {
        id: "google-calendar",
        label: "Google Calendar",
        category: "Google",
        auth: Auth::OAuth2 {
            client_id_env: "GOOGLE_OAUTH_CLIENT_ID",
            client_secret_env: "GOOGLE_OAUTH_CLIENT_SECRET",
            callback_path: "/api/connectors/google/callback",
            scopes: "https://www.googleapis.com/auth/calendar",
        },
        setup_note: "Enable the Google Calendar API on the GCP project (shares the one Google OAuth client).",
        docs: "https://console.cloud.google.com/apis/library/calendar-json.googleapis.com",
    },
    Provider {
        id: "google-drive",
        label: "Google Drive",
        category: "Google",
        auth: Auth::OAuth2 {
            client_id_env: "GOOGLE_OAUTH_CLIENT_ID",
            client_secret_env: "GOOGLE_OAUTH_CLIENT_SECRET",
            callback_path: "/api/connectors/google/callback",
            scopes: "https://www.googleapis.com/auth/drive",
        },
        setup_note: "Enable the Google Drive API on the GCP project (shares the one Google OAuth client).",
        docs: "https://console.cloud.google.com/apis/library/drive.googleapis.com",
    },
    Provider {
        id: "google-admin",
        label: "Google Admin",
        category: "Google",
        auth: Auth::OAuth2 {
            client_id_env: "GOOGLE_OAUTH_CLIENT_ID",
            client_secret_env: "GOOGLE_OAUTH_CLIENT_SECRET",
            callback_path: "/api/connectors/google/callback",
            scopes: "https://www.googleapis.com/auth/admin.directory.user.readonly https://www.googleapis.com/auth/admin.directory.group.readonly",
        },
        setup_note: "Needs a Workspace super-admin. Enable the Admin SDK API and consent the admin.directory scopes as the super-admin, or use a service account with domain-wide delegation.",
        docs: "https://console.cloud.google.com/apis/library/admin.googleapis.com",
    },
    Provider {
        id: "slack",
        label: "Slack",
        category: "Chat",
        auth: Auth::OAuth2 {
            client_id_env: "SLACK_CLIENT_ID",
            client_secret_env: "SLACK_CLIENT_SECRET",
            callback_path: "/api/connectors/slack/callback",
            scopes: "channels:read chat:write channels:history",
        },
        setup_note: "Create a Slack app, add the redirect URI below, and paste its client id + secret.",
        docs: "https://api.slack.com/apps",
    },
];

fn provider(id: &str) -> Option<&'static Provider> {
    REGISTRY.iter().find(|p| p.id == id)
}

/// The env-var NAMES a provider needs, in paste order. OAuth needs two (client
/// id + secret); ApiKey needs one.
fn env_keys(p: &Provider) -> Vec<&'static str> {
    match p.auth {
        Auth::ApiKey { key_env } => vec![key_env],
        Auth::OAuth2 {
            client_id_env,
            client_secret_env,
            ..
        } => vec![client_id_env, client_secret_env],
    }
}

/// This server's origin for redirect URIs, canonical-port so it is right in
/// both the laptop (8824) and the container (8822) with no build flag.
fn origin() -> String {
    format!("https://localhost:{}", canonical_port())
}

/// Does a provider have an OAuth token on disk? Tokens live under
/// `~/.amux/connectors/<provider>/` (chmod-600 files the broker writes). Gmail's
/// legacy tokens under `~/.amux/gmail-tokens/` also count the Google connectors
/// as authorized, since they share the client.
fn has_token(p: &Provider) -> bool {
    let home = amux_home();
    let dir = home.join("connectors").join(p.id);
    let any_json = |d: &std::path::Path| -> bool {
        std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "json"))
            })
            .unwrap_or(false)
    };
    if any_json(&dir) {
        return true;
    }
    if p.category == "Google" && any_json(&home.join("gmail-tokens")) {
        return true;
    }
    false
}

/// Non-empty value of an env key, server.env first (the write target), then the
/// process env — the same precedence config.rs uses. Returns the VALUE, only for
/// presence/masking; never emitted raw.
fn env_val(file_env: &std::collections::BTreeMap<String, String>, key: &str) -> Option<String> {
    file_env
        .get(key)
        .cloned()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))
}

/// GET /api/connectors — the registry with per-provider status. `?worker=` is
/// accepted for parity with the scope explain link but the status here is
/// global (credential presence + token); per-scope enablement is the scope
/// read.
async fn list() -> Response {
    let file_env = parse_env_file(&amux_home().join("server.env"));
    let items: Vec<Value> = REGISTRY
        .iter()
        .map(|p| {
            let keys = env_keys(p);
            let key_status: Vec<Value> = keys
                .iter()
                .map(|k| {
                    let v = env_val(&file_env, k);
                    json!({
                        "name": k,
                        "set": v.is_some(),
                        "masked": v.as_deref().map(super::settings::mask_secret),
                    })
                })
                .collect();
            let all_creds_set = key_status.iter().all(|k| k["set"].as_bool().unwrap_or(false));
            let (kind, oauth) = match p.auth {
                Auth::ApiKey { .. } => ("apikey", Value::Null),
                Auth::OAuth2 {
                    callback_path,
                    scopes,
                    ..
                } => (
                    "oauth2",
                    json!({
                        "redirect_uri": format!("{}{}", origin(), callback_path),
                        "scopes": scopes,
                    }),
                ),
            };
            // Status ladder, most-blocked first.
            let status = if !all_creds_set {
                "needs_credentials"
            } else if kind == "oauth2" && !has_token(p) {
                "needs_auth"
            } else {
                "connected"
            };
            json!({
                "id": p.id,
                "label": p.label,
                "category": p.category,
                "auth": kind,
                "oauth": oauth,
                "env_keys": key_status,
                "status": status,
                "setup_note": p.setup_note,
                "docs": p.docs,
            })
        })
        .collect();
    Json(json!({
        "connectors": items,
        "origin": origin(),
        "note": "Paste credential VALUES here; they are written to ~/.amux/server.env and never returned. Set scope (global/group/worker) via the Scope tab or the per-connector scope control (PUT /api/scope, capability=connectors).",
    }))
    .into_response()
}

/// POST /api/connectors/{id}/credentials — Ethan pastes this provider's
/// key(s)/secret(s). Body: `{ "<ENV_NAME>": "<value>", ... }`, restricted to the
/// env keys THIS provider declares (a paste for one provider can never write an
/// arbitrary env key). Values go to server.env and are redacted from the log and
/// the response.
async fn set_credentials(Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let Some(p) = provider(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": format!("unknown connector '{id}'")}))).into_response();
    };
    let allowed = env_keys(p);
    let Some(obj) = body.as_object() else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "body must be a JSON object of {ENV_NAME: value}"}))).into_response();
    };
    let home = amux_home();
    let mut written: Vec<String> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    for (k, v) in obj {
        if !allowed.contains(&k.as_str()) {
            rejected.push(k.clone());
            continue;
        }
        let val = v.as_str().unwrap_or("").trim();
        if val.is_empty() {
            rejected.push(k.clone());
            continue;
        }
        if let Err(e) = super::settings::set_server_env_key(&home, k, val) {
            tracing::warn!("connector_credentials: write failed for {} key {}: {}", id, k, e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("write failed for {k}")}))).into_response();
        }
        written.push(k.clone());
    }
    // Redacted audit — names of keys written, NEVER the values (two-fixes rule:
    // grep `connector_credentials` to see who set what, without leaking it).
    tracing::info!(
        "connector_credentials: {} wrote {:?} to server.env (rejected {:?})",
        id,
        written,
        rejected
    );
    if written.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": "nothing written",
                "rejected": rejected,
                "expected_keys": allowed,
                "why": "only this connector's declared env keys are accepted, and values must be non-empty",
            })),
        )
            .into_response();
    }
    // server.env is read at startup (setdefault); a fresh paste needs the value
    // in the PROCESS env to take effect without a restart. Mirror it in-process
    // so a just-pasted key works immediately for this run (settings.rs does the
    // same for ANTHROPIC_API_KEY).
    let file_env = parse_env_file(&home.join("server.env"));
    for k in &written {
        if let Some(v) = file_env.get(k) {
            // SAFETY: single-threaded config mutation at request time, same
            // pattern settings.rs uses; value came from our own atomic write.
            std::env::set_var(k, v);
        }
    }
    Json(json!({
        "ok": true,
        "connector": id,
        "written": written,
        "rejected": rejected,
        "note": "stored in ~/.amux/server.env; restart is not required for this run. Values are never returned.",
    }))
    .into_response()
}

/// POST /api/connectors/{id}/auth — begin an OAuth grant. Returns the provider
/// consent URL to open (the tab pops it). For ApiKey providers there is nothing
/// to authorize. The token exchange (callback) is the OAuth-broker step
/// (AMUX-3192); until it lands this returns the URL plus the redirect to
/// register, so the flow is walkable as soon as the client creds are pasted.
async fn begin_auth(Path(id): Path<String>) -> Response {
    let Some(p) = provider(&id) else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": format!("unknown connector '{id}'")}))).into_response();
    };
    let file_env = parse_env_file(&amux_home().join("server.env"));
    match p.auth {
        Auth::ApiKey { .. } => Json(json!({
            "ok": true,
            "auth": "apikey",
            "note": "key-only connector: paste the API key, no browser grant needed",
        }))
        .into_response(),
        Auth::OAuth2 {
            client_id_env,
            callback_path,
            scopes,
            ..
        } => {
            let Some(client_id) = env_val(&file_env, client_id_env) else {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({
                        "ok": false,
                        "error": "client credentials not set",
                        "need_env": client_id_env,
                        "how": "paste the OAuth client id + secret first (this connector's credentials form)",
                    })),
                )
                    .into_response();
            };
            let redirect_uri = format!("{}{}", origin(), callback_path);
            let auth_base = if p.category == "Google" {
                "https://accounts.google.com/o/oauth2/auth"
            } else {
                "https://slack.com/oauth/v2/authorize"
            };
            let url = format!(
                "{auth_base}?response_type=code&access_type=offline&prompt=consent&client_id={}&redirect_uri={}&scope={}",
                urlencode(&client_id),
                urlencode(&redirect_uri),
                urlencode(scopes),
            );
            Json(json!({
                "ok": true,
                "auth": "oauth2",
                "authorize_url": url,
                "redirect_uri": redirect_uri,
                "note": "register redirect_uri on the OAuth client, then open authorize_url to grant. Token exchange lands with the OAuth broker (AMUX-3192).",
            }))
            .into_response()
        }
    }
}

/// Minimal percent-encoding for query values (mirrors gmail_auth's urlencode).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// OAuth callback landing. The full code->token exchange is the broker step
/// (AMUX-3192); for now it acknowledges the redirect so a misregistered URI is
/// diagnosable rather than a blank page, and names what is still to come.
async fn callback(Path(provider_family): Path<String>, _headers: HeaderMap) -> Response {
    tracing::info!("connector_oauth_callback: hit for family {}", provider_family);
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "family": provider_family,
            "note": "redirect reached this server (URI is registered correctly). Code->token exchange lands with the OAuth broker, AMUX-3192.",
        })),
    )
        .into_response()
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/connectors", get(list))
        .route("/api/connectors/{id}/credentials", post(set_credentials))
        .route("/api/connectors/{id}/auth", post(begin_auth))
        .route("/api/connectors/{family}/callback", get(callback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_are_unique_and_have_env_keys() {
        let mut seen = std::collections::HashSet::new();
        for p in REGISTRY {
            assert!(seen.insert(p.id), "duplicate connector id {}", p.id);
            assert!(!env_keys(p).is_empty(), "{} declares no env keys", p.id);
        }
    }

    #[test]
    fn oauth_providers_expose_a_canonical_port_redirect() {
        // The redirect must follow canonical_port, never a hardcoded 8822 (the
        // exact bug that dead-ended the Gmail flow). Assert the port matches.
        let want = format!(":{}", canonical_port());
        for p in REGISTRY {
            if let Auth::OAuth2 { callback_path, .. } = p.auth {
                let uri = format!("{}{}", origin(), callback_path);
                assert!(uri.contains(&want), "{} redirect {} lost canonical port", p.id, uri);
                assert!(!uri.contains(":8822") || want == ":8822", "{} pins retired 8822", p.id);
            }
        }
    }

    #[test]
    fn google_connectors_share_one_oauth_client() {
        // The design's premise: one client, many APIs. If these ever diverge,
        // the setup checklist (one redirect, one consent) silently breaks.
        let google: Vec<_> = REGISTRY.iter().filter(|p| p.category == "Google").collect();
        assert!(google.len() >= 4);
        for p in &google {
            if let Auth::OAuth2 { client_id_env, callback_path, .. } = p.auth {
                assert_eq!(client_id_env, "GOOGLE_OAUTH_CLIENT_ID", "{} uses a different client", p.id);
                assert_eq!(callback_path, "/api/connectors/google/callback");
            }
        }
    }
}
