//! Secret metadata store — tracks purpose, ownership, rotation schedule
//!
//! Enables querying "what is this secret for?" and "when should it be rotated?"
//! Separate from secret values to keep concerns clean.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub id: i64,
    pub secret_path: String,
    pub service_name: String,
    pub purpose: String,
    pub used_by: Vec<String>, // Parsed from JSON
    pub owner: Option<String>,
    pub rotation_days: Option<i32>,
    pub last_rotated: Option<DateTime<Utc>>,
}

/// Get metadata for a specific secret
pub async fn get_metadata(pool: &SqlitePool, secret_path: &str) -> Result<Option<SecretMetadata>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, secret_path, service_name, purpose, used_by, owner, rotation_days, last_rotated
        FROM secret_metadata
        WHERE secret_path = ?
        "#,
        secret_path
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| SecretMetadata {
        id: r.id,
        secret_path: r.secret_path,
        service_name: r.service_name,
        purpose: r.purpose,
        used_by: serde_json::from_str(&r.used_by).unwrap_or_default(),
        owner: r.owner,
        rotation_days: r.rotation_days,
        last_rotated: r.last_rotated.and_then(|ts| {
            DateTime::parse_from_rfc3339(&ts)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
    }))
}

/// List all secret metadata
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<SecretMetadata>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, secret_path, service_name, purpose, used_by, owner, rotation_days, last_rotated
        FROM secret_metadata
        ORDER BY service_name, secret_path
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SecretMetadata {
            id: r.id,
            secret_path: r.secret_path,
            service_name: r.service_name,
            purpose: r.purpose,
            used_by: serde_json::from_str(&r.used_by).unwrap_or_default(),
            owner: r.owner,
            rotation_days: r.rotation_days,
            last_rotated: r.last_rotated.and_then(|ts| {
                DateTime::parse_from_rfc3339(&ts)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
        })
        .collect())
}

/// Create or update secret metadata
pub async fn set_metadata(pool: &SqlitePool, metadata: &SecretMetadata) -> Result<(), sqlx::Error> {
    let used_by_json = serde_json::to_string(&metadata.used_by).unwrap_or_else(|_| "[]".to_string());
    let last_rotated = metadata
        .last_rotated
        .map(|dt| dt.to_rfc3339());

    sqlx::query!(
        r#"
        INSERT INTO secret_metadata
          (secret_path, service_name, purpose, used_by, owner, rotation_days, last_rotated)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(secret_path) DO UPDATE SET
          service_name = excluded.service_name,
          purpose = excluded.purpose,
          used_by = excluded.used_by,
          owner = excluded.owner,
          rotation_days = excluded.rotation_days,
          last_rotated = excluded.last_rotated,
          updated_at = CURRENT_TIMESTAMP
        "#,
        metadata.secret_path,
        metadata.service_name,
        metadata.purpose,
        used_by_json,
        metadata.owner,
        metadata.rotation_days,
        last_rotated
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Check if a secret needs rotation based on last_rotated and rotation_days
pub fn needs_rotation(metadata: &SecretMetadata) -> bool {
    match (metadata.last_rotated, metadata.rotation_days) {
        (Some(last_rotated), Some(rotation_days)) => {
            let now = Utc::now();
            let days_since = now
                .signed_duration_since(last_rotated)
                .num_days();
            days_since >= rotation_days as i64
        }
        _ => false,
    }
}

/// Calculate days until rotation is due
pub fn days_until_rotation(metadata: &SecretMetadata) -> Option<i64> {
    match (metadata.last_rotated, metadata.rotation_days) {
        (Some(last_rotated), Some(rotation_days)) => {
            let now = Utc::now();
            let days_since = now
                .signed_duration_since(last_rotated)
                .num_days();
            let days_until = rotation_days as i64 - days_since;
            Some(days_until.max(0))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_needs_rotation() {
        let metadata = SecretMetadata {
            id: 1,
            secret_path: "test.key".to_string(),
            service_name: "test-service".to_string(),
            purpose: "test".to_string(),
            used_by: vec![],
            owner: None,
            rotation_days: Some(90),
            last_rotated: Some(Utc::now() - chrono::Duration::days(91)),
        };

        assert!(needs_rotation(&metadata));
    }

    #[test]
    fn test_does_not_need_rotation() {
        let metadata = SecretMetadata {
            id: 1,
            secret_path: "test.key".to_string(),
            service_name: "test-service".to_string(),
            purpose: "test".to_string(),
            used_by: vec![],
            owner: None,
            rotation_days: Some(90),
            last_rotated: Some(Utc::now() - chrono::Duration::days(30)),
        };

        assert!(!needs_rotation(&metadata));
    }
}
