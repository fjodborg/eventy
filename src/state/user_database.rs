use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Database tracking all verified users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDatabase {
    /// Schema version for migrations
    pub version: u32,

    /// Last update timestamp
    pub last_updated: u64,

    /// Map of Discord ID (as string) to tracked user
    pub users: HashMap<String, TrackedUser>,
}

impl Default for UserDatabase {
    fn default() -> Self {
        Self {
            version: 3,
            last_updated: current_timestamp(),
            users: HashMap::new(),
        }
    }
}

impl UserDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from a JSON file, or create new if not exists
    pub async fn load(path: &str) -> crate::error::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                // Parse as generic JSON first to check version/structure
                let mut value: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                    crate::error::BotError::ConfigParse {
                        path: path.to_string(),
                        source: e,
                    }
                })?;

                // Check if migration is needed
                let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);

                if version < 3 {
                    // Perform migration
                    tracing::info!("Migrating user database from version {} to 3", version);

                    if let Some(users) = value.get_mut("users").and_then(|u| u.as_object_mut()) {
                        for (_, user_value) in users.iter_mut() {
                            if let Some(user_obj) = user_value.as_object_mut() {
                                // Migration 1 -> 2: verification_id -> verification_ids
                                if !user_obj.contains_key("verification_ids") {
                                    let mut verification_ids = serde_json::Map::new();

                                    // Try to get old verification_id
                                    if let Some(vid) =
                                        user_obj.get("verification_id").and_then(|v| v.as_str())
                                    {
                                        // Try to guess season from seasons list or default to "2024E" (legacy)
                                        let season = user_obj
                                            .get("seasons")
                                            .and_then(|s| s.as_array())
                                            .and_then(|a| a.first())
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("2024E"); // Fallback for very old data

                                        verification_ids.insert(
                                            season.to_string(),
                                            serde_json::Value::String(vid.to_string()),
                                        );
                                    }

                                    user_obj.insert(
                                        "verification_ids".to_string(),
                                        serde_json::Value::Object(verification_ids),
                                    );
                                }

                                // Migration 2 -> 3: Remove seasons (it's redundant with verification_ids keys)
                                // We don't strictly need to remove it for parsing to succeed if we use #[serde(ignore_unknown)]
                                // or just let it be ignored, but let's clean it up.
                                user_obj.remove("seasons");
                                user_obj.remove("verification_id"); // Clean up old field
                            }
                        }
                    }

                    // Update version
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("version".to_string(), serde_json::Value::Number(3.into()));
                    }
                }

                // Now parse into struct
                serde_json::from_value(value).map_err(|e| crate::error::BotError::ConfigParse {
                    path: path.to_string(),
                    source: e,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(crate::error::BotError::StateLoad {
                path: path.to_string(),
                source: e,
            }),
        }
    }

    /// Save to a JSON file atomically
    pub async fn save(&self, path: &str) -> crate::error::Result<()> {
        let content = serde_json::to_string_pretty(self)?;

        // Write to temp file first, then rename for atomicity
        let temp_path = format!("{}.tmp", path);
        tokio::fs::write(&temp_path, &content).await.map_err(|e| {
            crate::error::BotError::StateSave {
                path: path.to_string(),
                source: e,
            }
        })?;

        tokio::fs::rename(&temp_path, path).await.map_err(|e| {
            crate::error::BotError::StateSave {
                path: path.to_string(),
                source: e,
            }
        })?;

        Ok(())
    }

    /// Find a user by their Discord ID
    pub fn find_by_discord_id(&self, discord_id: &str) -> Option<&TrackedUser> {
        self.users.get(discord_id)
    }

    /// Find a user by their verification ID (UUID)
    pub fn find_by_verification_id(&self, verification_id: &str) -> Option<&TrackedUser> {
        self.users.values().find(|u| {
            u.verification_ids
                .values()
                .any(|vid| vid == verification_id)
        })
    }

    /// Check if a Discord user is verified
    pub fn is_verified(&self, discord_id: &str) -> bool {
        self.users
            .get(discord_id)
            .map(|u| u.verification_status == VerificationStatus::Verified)
            .unwrap_or(false)
    }

    /// Add or update a tracked user
    pub fn upsert_user(&mut self, user: TrackedUser) {
        self.users.insert(user.discord_id.clone(), user);
        self.last_updated = current_timestamp();
    }

    /// Get all users
    pub fn get_all_users(&self) -> Vec<&TrackedUser> {
        self.users.values().collect()
    }

    /// Get users by season
    pub fn get_users_by_season(&self, season_id: &str) -> Vec<&TrackedUser> {
        self.users
            .values()
            .filter(|u| u.verification_ids.contains_key(season_id))
            .collect()
    }

    /// Get user count
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Export as JSON bytes (for download)
    pub fn export(&self) -> crate::error::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self).map_err(|e| e.into())
    }
}

/// A tracked user in the database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedUser {
    /// Discord user ID (snowflake as string)
    pub discord_id: String,

    /// Verification IDs (Season ID -> UUID)
    pub verification_ids: HashMap<String, String>,

    /// Display name from the season file
    pub display_name: String,

    /// When the user was verified (Unix timestamp)
    pub verified_at: u64,

    /// Special roles (Bestyrelse, Korleder, etc.)
    pub special_roles: Vec<String>,

    /// All currently assigned Discord roles
    pub current_roles: Vec<String>,

    /// Verification status
    pub verification_status: VerificationStatus,

    /// Last time the user was seen (Unix timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,

    /// Optional notes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl TrackedUser {
    /// Create a new tracked user after successful verification
    pub fn new(
        discord_id: String,
        verification_id: String,
        season_id: String,
        display_name: String,
        special_roles: Vec<String>,
    ) -> Self {
        let mut verification_ids = HashMap::new();
        verification_ids.insert(season_id, verification_id);

        Self {
            discord_id,
            verification_ids,
            display_name,
            verified_at: current_timestamp(),
            special_roles,
            current_roles: Vec::new(),
            verification_status: VerificationStatus::Verified,
            last_seen: Some(current_timestamp()),
            notes: None,
        }
    }

    /// Update last seen timestamp
    pub fn update_last_seen(&mut self) {
        self.last_seen = Some(current_timestamp());
    }

    /// Add a role to current roles
    pub fn add_role(&mut self, role: &str) {
        if !self.current_roles.contains(&role.to_string()) {
            self.current_roles.push(role.to_string());
        }
    }

    /// Remove a role from current roles
    pub fn remove_role(&mut self, role: &str) {
        self.current_roles.retain(|r| r != role);
    }

    /// Add a verification ID for a season
    pub fn add_verification_id(&mut self, season_id: &str, verification_id: &str) {
        self.verification_ids
            .insert(season_id.to_string(), verification_id.to_string());
    }
}

/// Verification status for a user
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VerificationStatus {
    Pending,
    Verified,
    Revoked,
    Expired,
}

/// Shared user database type
pub type SharedUserDatabase = Arc<tokio::sync::RwLock<UserDatabase>>;

pub fn create_shared_user_database(db: UserDatabase) -> SharedUserDatabase {
    Arc::new(tokio::sync::RwLock::new(db))
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracked_user_creation() {
        let user = TrackedUser::new(
            "123456789".to_string(),
            "test-uuid".to_string(),
            "2025E".to_string(),
            "Test User".to_string(),
            vec!["Bestyrelse".to_string()],
        );

        assert_eq!(user.discord_id, "123456789");
        assert_eq!(user.verification_status, VerificationStatus::Verified);
        assert!(user.verification_ids.contains_key("2025E"));
        assert_eq!(
            user.verification_ids.get("2025E"),
            Some(&"test-uuid".to_string())
        );
    }

    #[test]
    fn test_user_database_operations() {
        let mut db = UserDatabase::new();

        let user = TrackedUser::new(
            "123".to_string(),
            "uuid-1".to_string(),
            "2025E".to_string(),
            "Test".to_string(),
            vec![],
        );

        db.upsert_user(user);

        assert!(db.is_verified("123"));
        assert!(db.find_by_discord_id("123").is_some());
        assert!(db.find_by_verification_id("uuid-1").is_some());
    }
}
