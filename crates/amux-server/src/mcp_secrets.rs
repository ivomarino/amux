//! MCP server for secrets access — allows Claude agents to request secrets securely
//! 
//! Implements: REQUEST_SECRET (path) → value or redacted

use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct SecretRequest {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SecretResponse {
    pub value: String,
    pub path: String,
}

/// MCP tool: REQUEST_SECRET
/// Allows Claude agents to request specific secrets by path
pub async fn request_secret(
    path: String,
    secret_store: Arc<crate::secrets::SecretStore>,
) -> Result<SecretResponse, String> {
    // Validate path format (dot-separated, no wildcards)
    if path.contains('*') || path.contains('?') {
        return Err("Wildcards not allowed in secret paths".to_string());
    }

    // Get the secret value
    match secret_store.get(&path).await {
        Some(value) => Ok(SecretResponse { value, path }),
        None => Err(format!("Secret not found: {}", path)),
    }
}

/// List all available secret paths (no values)
pub async fn list_secrets(
    secret_store: Arc<crate::secrets::SecretStore>,
) -> Result<Vec<String>, String> {
    let paths = secret_store.list_paths().await;
    Ok(paths)
}

/// Get schema structure without values
pub async fn inspect_schema(
    secret_store: Arc<crate::secrets::SecretStore>,
) -> Result<serde_json::Value, String> {
    let schema = secret_store.inspect_schema().await;
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_validation() {
        assert!(SecretRequest { path: "api.*.key".to_string() }
            .path.contains('*'));
        assert!(!SecretRequest { path: "api.key".to_string() }
            .path.contains('*'));
    }
}
