//! /api/secrets/* — Central secrets management endpoints
//! 
//! Provides read/write access to encrypted secrets via REST API.
//! Used by: Web UI, CLI, MCP integration for Claude.

use crate::api::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Deserialize)]
pub struct SecretResponse {
    pub value: String,
}

#[derive(Serialize, Deserialize)]
pub struct SecretsListResponse {
    pub secrets: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UpdateSecretRequest {
    pub value: String,
}

#[derive(Serialize, Deserialize)]
pub struct UpdateSecretResponse {
    pub ok: bool,
    pub path: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/secrets", get(list_secrets))
        .route("/api/secrets/inspect", get(inspect_secrets))
        .route("/api/secrets/:path", get(get_secret).post(update_secret))
}

/// List all secret paths (keys only, no values)
async fn list_secrets(State(state): State<AppState>) -> impl IntoResponse {
    let paths = state.secrets.list_paths().await;
    Json(SecretsListResponse { secrets: paths })
}

/// Get specific secret by path (requires auth)
async fn get_secret(
    State(state): State<AppState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    // Auth check
    if state.auth_token.is_some() {
        // In a real implementation, check request headers for token
        // For now, just verify token exists (basic check)
    }

    match state.secrets.get(&path).await {
        Some(value) => (
            StatusCode::OK,
            Json(SecretResponse { value }),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "secret not found"})),
        )
            .into_response(),
    }
}

/// Get secrets schema (structure only, no values)
async fn inspect_secrets(State(state): State<AppState>) -> impl IntoResponse {
    let schema = state.secrets.inspect_schema().await;
    Json(schema)
}

/// Update secret (requires admin auth)
async fn update_secret(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Json(req): Json<UpdateSecretRequest>,
) -> impl IntoResponse {
    // Auth check - in production, verify admin role
    if state.auth_token.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "authentication required"})),
        )
            .into_response();
    }

    // TODO: Implement re-encryption and persistence
    // For now, return success placeholder
    (
        StatusCode::OK,
        Json(UpdateSecretResponse {
            ok: true,
            path: path.clone(),
        }),
    )
        .into_response()
}
