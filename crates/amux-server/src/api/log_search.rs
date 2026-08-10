//! GET /api/log-search?q=&max= — grep session log files.
//!
//! Returns `{matches: {session_name: [{line, text}, ...]}, q}`.

use axum::extract::Query;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_max")]
    max: usize,
}

fn default_max() -> usize {
    50
}

fn logs_dir() -> PathBuf {
    let home = std::env::var("AMUX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".amux")
        });
    home.join("logs")
}

pub async fn search(Query(q): Query<SearchQuery>) -> Response {
    let query = q.q.trim().to_string();
    if query.is_empty() {
        return Json(json!({"matches": {}})).into_response();
    }
    let max_per = q.max.clamp(1, 500);

    let result = tokio::task::spawn_blocking(move || {
        let ql = query.to_lowercase();
        let ansi_re = regex::Regex::new(
            r"\x1b\[[0-9;?]*[a-zA-Z]",
        )
        .unwrap();

        let mut matches: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        let dir = logs_dir();
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "log")
                    .unwrap_or(false)
            })
            .collect();
        files.sort_by_key(|e| e.file_name());

        for entry in files {
            let path = entry.path();
            let sess_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let mut hits = Vec::new();
            for (i, raw) in text.lines().enumerate() {
                if raw.to_lowercase().contains(&ql) {
                    let clean = ansi_re.replace_all(raw, "").trim().to_string();
                    if !clean.is_empty() {
                        let truncated = if clean.len() > 500 {
                            clean[..500].to_string()
                        } else {
                            clean
                        };
                        hits.push(json!({"line": i + 1, "text": truncated}));
                        if hits.len() >= max_per {
                            break;
                        }
                    }
                }
            }
            if !hits.is_empty() {
                matches.insert(sess_name, hits);
            }
        }
        (matches, query)
    })
    .await;

    match result {
        Ok((matches, query)) => Json(json!({"matches": matches, "q": query})).into_response(),
        Err(e) => {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": e.to_string()}).to_string(),
            )
                .into_response()
        }
    }
}
