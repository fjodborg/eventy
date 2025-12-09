use serde::{Deserialize, Serialize};

/// Configuration for a season (e.g., 2025E for Fall 2025)
/// Contains the list of users who can verify for this season
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonConfig {
    /// Unique identifier for the season (e.g., "2025E")
    pub season_id: String,

    /// Human-readable name (e.g., "2025 Efterår")
    #[serde(default)]
    pub name: String,

    /// Whether this season is currently active for verification
    #[serde(default = "default_active")]
    pub active: bool,

    /// List of users in this season
    pub users: Vec<SeasonUser>,
}

fn default_active() -> bool {
    true
}

/// A user entry in a season file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonUser {
    /// User's display name
    #[serde(rename = "Name")]
    pub name: String,

    /// Verification ID (UUID) - NOT the Discord ID
    #[serde(rename = "DiscordId")]
    pub id: String,

    /// Optional email for reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl SeasonConfig {
    /// Load a season config from a JSON file
    pub fn load_from_file(path: &str) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| crate::error::BotError::ConfigLoad {
            path: path.to_string(),
            source: e,
        })?;

        // Try parsing as SeasonConfig first
        if let Ok(config) = serde_json::from_str::<SeasonConfig>(&content) {
            return Ok(config);
        }

        // Fall back to parsing as array of users (legacy format)
        let users: Vec<SeasonUser> = serde_json::from_str(&content).map_err(|e| {
            crate::error::BotError::ConfigParse {
                path: path.to_string(),
                source: e,
            }
        })?;

        // Extract season_id from filename (e.g., "2025E.json" -> "2025E")
        let season_id = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(SeasonConfig {
            season_id: season_id.clone(),
            name: season_id,
            active: true,
            users,
        })
    }

    /// Find a user by their verification ID
    pub fn find_user_by_id(&self, verification_id: &str) -> Option<&SeasonUser> {
        self.users.iter().find(|u| u.id == verification_id)
    }

    /// Get the number of users in this season
    pub fn user_count(&self) -> usize {
        self.users.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_legacy_format() {
        let json = r#"[
            {"Name": "Test User", "DiscordId": "test-uuid-123"}
        ]"#;

        let users: Vec<SeasonUser> = serde_json::from_str(json).unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "Test User");
        assert_eq!(users[0].id, "test-uuid-123");
    }

    #[test]
    fn test_parse_new_format() {
        let json = r#"{
            "season_id": "2025E",
            "name": "2025 Efterår",
            "active": true,
            "users": [
                {"Name": "Test User", "DiscordId": "test-uuid-123"}
            ]
        }"#;

        let config: SeasonConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.season_id, "2025E");
        assert_eq!(config.users.len(), 1);
    }
}
