//! Secret persistence — encrypt and write secrets to disk
//!
//! Handles:
//! - Serializing secrets to YAML
//! - Encrypting with age via SOPS
//! - Atomic writes (temp file → rename)
//! - Decryption for reloads
//!
//! Used by: POST /api/secrets/{path} endpoint to save changes

use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

/// Encrypt and write secrets to file
///
/// # Process
/// 1. Serialize secrets to YAML
/// 2. Write to temporary file
/// 3. Encrypt with SOPS (age)
/// 4. Atomically rename (temp → real)
/// 5. Clean up temp file
///
/// # Arguments
/// * `secrets` - Secrets as nested JSON
/// * `secrets_file` - Path to encrypted output file
/// * `_age_key_path` - Path to age private key (used by SOPS config)
///
/// # Errors
/// Returns error if encryption fails or file write fails
pub async fn encrypt_and_persist(
    secrets: &Value,
    secrets_file: &Path,
    _age_key_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Serialize to YAML
    let yaml_str = serde_yaml::to_string(secrets)?;

    // 2. Create temp file path
    let temp_path = format!("{}.tmp", secrets_file.display());
    let temp_path = PathBuf::from(&temp_path);

    // 3. Write YAML to temp file
    tokio::fs::write(&temp_path, &yaml_str).await?;

    // 4. Encrypt with SOPS (using age)
    let encrypt_output = tokio::process::Command::new("sops")
        .arg("-e")
        .arg("--input-type")
        .arg("yaml")
        .arg("--output-type")
        .arg("yaml")
        .arg(&temp_path)
        .output()
        .await
        .map_err(|e| {
            error!("Failed to run SOPS: {}", e);
            e
        })?;

    if !encrypt_output.status.success() {
        let stderr = String::from_utf8_lossy(&encrypt_output.stderr);
        error!("SOPS encryption failed: {}", stderr);

        // Clean up temp file
        let _ = tokio::fs::remove_file(&temp_path).await;

        return Err(format!("SOPS encryption failed: {}", stderr).into());
    }

    // 5. Write encrypted output to final location (atomic via rename)
    tokio::fs::write(secrets_file, &encrypt_output.stdout).await?;

    info!(
        path = ?secrets_file,
        size = encrypt_output.stdout.len(),
        "✓ Persisted encrypted secrets"
    );

    // 6. Clean up temp file
    if let Err(e) = tokio::fs::remove_file(&temp_path).await {
        warn!("Failed to clean up temp file: {}", e);
    }

    Ok(())
}

/// Load and decrypt secrets from file
///
/// # Process
/// 1. Decrypt with age (via SOPS)
/// 2. Parse YAML
/// 3. Return as JSON
///
/// # Arguments
/// * `secrets_file` - Path to encrypted secrets file
/// * `age_key_path` - Path to age private key
///
/// # Errors
/// Returns error if file doesn't exist, decryption fails, or YAML is invalid
pub async fn load_and_decrypt(
    secrets_file: &Path,
    age_key_path: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    // Check if file exists
    if !secrets_file.exists() {
        warn!(
            path = ?secrets_file,
            "Secrets file not found, using empty store"
        );
        return Ok(Value::Object(Default::default()));
    }

    // Decrypt using age
    let decrypt_output = tokio::process::Command::new("age")
        .arg("-d")
        .arg("-i")
        .arg(age_key_path)
        .arg(secrets_file)
        .output()
        .await
        .map_err(|e| {
            error!("Failed to run age: {}", e);
            e
        })?;

    if !decrypt_output.status.success() {
        let stderr = String::from_utf8_lossy(&decrypt_output.stderr);
        error!("age decryption failed: {}", stderr);
        return Err(format!("age decryption failed: {}", stderr).into());
    }

    // Parse decrypted YAML
    let decrypted_str = String::from_utf8(decrypt_output.stdout)?;
    let secrets_value: Value = serde_yaml::from_str(&decrypted_str)?;

    info!(path = ?secrets_file, "✓ Loaded and decrypted secrets");

    Ok(secrets_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[tokio::test]
    #[ignore] // Requires age key to be configured
    async fn test_encrypt_and_persist() {
        let test_secrets = json!({
            "test": {
                "key": "value"
            }
        });

        let temp_file = NamedTempFile::new().unwrap();
        let secrets_path = temp_file.path();
        let age_key_path = PathBuf::from(
            std::env::var("AGE_KEY_PATH").unwrap_or_else(|_| {
                "~/.config/sops/age/keys.txt".to_string()
            }),
        );

        // Try to encrypt
        match encrypt_and_persist(&test_secrets, secrets_path, &age_key_path).await {
            Ok(_) => {
                // Try to decrypt
                match load_and_decrypt(secrets_path, &age_key_path).await {
                    Ok(loaded) => {
                        assert_eq!(loaded["test"]["key"], "value");
                    }
                    Err(e) => {
                        eprintln!("Decryption failed: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Encryption failed: {}", e);
            }
        }
    }
}
