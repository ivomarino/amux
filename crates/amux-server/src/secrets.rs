//! Central secrets store — loads and decrypts amux-secrets.yaml
//!
//! At server startup:
//! 1. Read secrets/amux-secrets.yaml (encrypted with age)
//! 2. Decrypt using age key from ~/.config/sops/age/keys.txt
//! 3. Parse YAML into in-memory SecretStore
//! 4. Expose via environment variables and API
//!
//! Usage:
//! ```
//! let secrets = SecretStore::load().await?;
//! let openai_key = secrets.get("external_services.openai.api_key");
//! ```

pub mod persist;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Central secrets store — loads, decrypts, and caches secrets
pub struct SecretStore {
    /// Decrypted secrets cache (nested JSON)
    secrets: Arc<RwLock<Value>>,
    /// Path to age private key
    age_key_path: PathBuf,
    /// Path to encrypted secrets file
    secrets_file: PathBuf,
}

impl SecretStore {
    /// Create a new SecretStore instance
    pub fn new(age_key_path: PathBuf, secrets_file: PathBuf) -> Self {
        Self {
            secrets: Arc::new(RwLock::new(Value::Object(Default::default()))),
            age_key_path,
            secrets_file,
        }
    }

    /// Load encrypted secrets file and decrypt with age
    pub async fn load(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Check if secrets file exists
        if !self.secrets_file.exists() {
            tracing::warn!(
                path = ?self.secrets_file,
                "Secrets file not found, using empty store"
            );
            return Ok(());
        }

        // Decrypt using age
        let output = tokio::process::Command::new("age")
            .arg("-d")
            .arg("-i")
            .arg(&self.age_key_path)
            .arg(&self.secrets_file)
            .output()
            .await
            .map_err(|e| {
                tracing::error!(error = ?e, "Failed to decrypt secrets with age");
                e
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("age decryption failed: {}", stderr).into());
        }

        // Parse decrypted YAML
        let decrypted_str = String::from_utf8(output.stdout)?;
        let secrets_value: Value = serde_yaml::from_str(&decrypted_str)?;

        // Cache the decrypted secrets
        let mut cache = self.secrets.write().await;
        *cache = secrets_value;

        tracing::info!("Secrets loaded and decrypted successfully");
        Ok(())
    }

    /// Get a secret by dot-separated path (e.g., "external_services.openai.api_key")
    pub async fn get(&self, path: &str) -> Option<String> {
        let cache = self.secrets.read().await;

        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &*cache;

        for part in parts {
            current = &current[part];
            if current.is_null() {
                return None;
            }
        }

        current.as_str().map(|s| s.to_string())
    }

    /// List all secret paths (keys only, no values)
    pub async fn list_paths(&self) -> Vec<String> {
        let cache = self.secrets.read().await;
        flatten_keys(&cache, "")
    }

    /// Get secrets as nested JSON (for inspection, no values)
    pub async fn inspect_schema(&self) -> Value {
        let cache = self.secrets.read().await;
        schema_only(&cache)
    }

    /// Load secrets into environment variables
    pub async fn load_env(&self) -> Result<(), Box<dyn std::error::Error>> {
        let paths = self.list_paths().await;
        
        for path in paths {
            if let Some(value) = self.get(&path).await {
                // Convert dot-separated path to ENV-style name
                let env_name = path.replace('.', "_").to_uppercase();
                std::env::set_var(&env_name, &value);
                
                // Also set specific known env vars
                match path.as_str() {
                    "external_services.openai.api_key" => {
                        std::env::set_var("OPENAI_API_KEY", &value);
                    }
                    "external_services.openai.organization" => {
                        std::env::set_var("OPENAI_ORG_ID", &value);
                    }
                    "oauth.google.client_id" => {
                        std::env::set_var("GOOGLE_OAUTH_CLIENT_ID", &value);
                    }
                    "oauth.google.client_secret" => {
                        std::env::set_var("GOOGLE_OAUTH_CLIENT_SECRET", &value);
                    }
                    _ => {}
                }
            }
        }

        tracing::info!("Secrets loaded into environment variables");
        Ok(())
    }

    /// Reload secrets from disk (for manual refresh)
    pub async fn reload(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.load().await?;
        self.load_env().await?;
        Ok(())
    }

    /// Update a single secret and persist to encrypted file
    ///
    /// # Process
    /// 1. Lock cache for write
    /// 2. Update value at path
    /// 3. Encrypt and write to disk
    /// 4. Keep cache updated
    ///
    /// # Arguments
    /// * `path` - Dot-separated path (e.g., "oauth.github.client_id")
    /// * `value` - Secret value to set
    ///
    /// # Errors
    /// Returns error if encryption fails or path is invalid
    pub async fn update_and_persist(
        &self,
        path: &str,
        value: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Lock cache for write
        let mut cache = self.secrets.write().await;

        // 2. Update value in nested structure
        Self::set_nested_value(&mut cache, path, value.clone())?;

        // 3. Encrypt and persist to disk
        persist::encrypt_and_persist(&cache, &self.secrets_file, &self.age_key_path).await?;

        // 4. Update env vars from new value
        let env_name = path.replace('.', "_").to_uppercase();
        std::env::set_var(&env_name, &value);

        tracing::info!(path = path, "✓ Updated and persisted secret");
        Ok(())
    }

    /// Set a value in nested JSON object by dot-separated path
    fn set_nested_value(obj: &mut Value, path: &str, value: String) -> Result<(), Box<dyn std::error::Error>> {
        let parts: Vec<&str> = path.split('.').collect();

        if parts.is_empty() {
            return Err("Path cannot be empty".into());
        }

        let mut current = obj;

        // Navigate/create intermediate objects
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // Last part: set the value
                current[part] = Value::String(value.clone());
            } else {
                // Intermediate part: navigate or create
                if !current[part].is_object() {
                    current[part] = Value::Object(Default::default());
                }
                current = &mut current[part];
            }
        }

        Ok(())
    }
}

/// Flatten nested JSON into dot-separated keys
fn flatten_keys(value: &Value, prefix: &str) -> Vec<String> {
    let mut keys = Vec::new();

    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };

                if val.is_object() || val.is_array() {
                    keys.extend(flatten_keys(val, &full_key));
                } else if !val.is_null() {
                    keys.push(full_key);
                }
            }
        }
        Value::Array(arr) => {
            for (idx, val) in arr.iter().enumerate() {
                let full_key = format!("{}[{}]", prefix, idx);
                keys.extend(flatten_keys(val, &full_key));
            }
        }
        _ => {
            if !value.is_null() && !prefix.is_empty() {
                keys.push(prefix.to_string());
            }
        }
    }

    keys
}

/// Extract schema (structure) without values
fn schema_only(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut result = serde_json::Map::new();
            for (key, val) in map {
                result.insert(key.clone(), schema_only(val));
            }
            Value::Object(result)
        }
        Value::Array(arr) => {
            Value::Array(
                arr.iter()
                    .map(schema_only)
                    .collect(),
            )
        }
        Value::String(_) => json!("<redacted>"),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flatten_keys() {
        let json = json!({
            "external_services": {
                "openai": {
                    "api_key": "sk-123",
                    "org": "org-456"
                }
            },
            "databases": {
                "postgres": {
                    "password": "secret"
                }
            }
        });

        let keys = flatten_keys(&json, "");
        assert!(keys.contains(&"external_services.openai.api_key".to_string()));
        assert!(keys.contains(&"databases.postgres.password".to_string()));
    }

    #[test]
    fn test_schema_only() {
        let json = json!({
            "api_key": "sk-123",
            "nested": {
                "password": "secret"
            }
        });

        let schema = schema_only(&json);
        assert_eq!(schema["api_key"], json!("<redacted>"));
        assert_eq!(schema["nested"]["password"], json!("<redacted>"));
    }
}
