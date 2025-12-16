use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::global_structure::{ChannelDefinition, ChannelPermissionLevel, ChannelType};

/// Season configuration (loaded from season.json)
/// Contains metadata and channel structure for a season
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonConfig {
    /// Human-readable name (e.g., "2025 Efterår")
    pub name: String,

    /// Whether this season is currently active for verification
    #[serde(default = "default_active")]
    pub active: bool,

    /// The member role for this season (e.g., "Medlem2025E")
    /// Users who verify for this season get this role
    /// If not specified, falls back to global default_member_role
    #[serde(default)]
    pub member_role: Option<String>,

    /// Channel definitions for this season's category
    #[serde(default)]
    pub channels: Vec<ChannelDefinition>,
}

fn default_active() -> bool {
    true
}

impl SeasonConfig {
    /// Load from a JSON file
    pub fn load_from_file(path: &str) -> crate::error::Result<Self> {
        let content =
            std::fs::read_to_string(path).map_err(|e| crate::error::BotError::ConfigLoad {
                path: path.to_string(),
                source: e,
            })?;

        serde_json::from_str(&content).map_err(|e| crate::error::BotError::ConfigParse {
            path: path.to_string(),
            source: e,
        })
    }
}

impl Default for SeasonConfig {
    fn default() -> Self {
        Self {
            name: "New Season".to_string(),
            active: true,
            member_role: None,
            channels: vec![ChannelDefinition {
                name: "general".to_string(),
                channel_type: ChannelType::Text,
                position: Some(0),
                role_permissions: {
                    let mut perms = HashMap::new();
                    perms.insert("Medlem".to_string(), ChannelPermissionLevel::ReadWrite);
                    perms
                },
                children: vec![],
            }],
        }
    }
}

/// A user entry in the users.json file (externally generated)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Load users from a JSON file (simple array format)
pub fn load_users_from_file(path: &str) -> crate::error::Result<Vec<SeasonUser>> {
    let content =
        std::fs::read_to_string(path).map_err(|e| crate::error::BotError::ConfigLoad {
            path: path.to_string(),
            source: e,
        })?;

    serde_json::from_str(&content).map_err(|e| crate::error::BotError::ConfigParse {
        path: path.to_string(),
        source: e,
    })
}

/// Combined season data (config + users, with season_id derived from directory)
#[derive(Debug, Clone)]
pub struct Season {
    /// Season ID (derived from directory name)
    pub season_id: String,

    /// Season configuration (from season.json)
    pub config: SeasonConfig,

    /// Users in this season (from users.json)
    pub users: Vec<SeasonUser>,
}

impl Season {
    /// Create a new Season from config and users
    pub fn new(season_id: String, config: SeasonConfig, users: Vec<SeasonUser>) -> Self {
        Self {
            season_id,
            config,
            users,
        }
    }

    /// Get the display name
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Check if the season is active
    pub fn is_active(&self) -> bool {
        self.config.active
    }

    /// Get channels
    pub fn channels(&self) -> &[ChannelDefinition] {
        &self.config.channels
    }

    /// Find a user by their verification ID
    pub fn find_user_by_id(&self, verification_id: &str) -> Option<&SeasonUser> {
        self.users.iter().find(|u| u.id == verification_id)
    }

    /// Get the number of users in this season
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// Get the member role for this season
    /// Falls back to "Medlem{season_id}" if not specified
    pub fn member_role(&self) -> String {
        self.config.member_role.clone().unwrap_or_else(|| format!("Medlem{}", self.season_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_season_config() {
        let json = r#"{
            "name": "2025 Efterår",
            "active": true,
            "channels": [
                {
                    "name": "general",
                    "type": "text",
                    "role_permissions": {
                        "Medlem": "readwrite"
                    }
                }
            ]
        }"#;

        let config: SeasonConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.name, "2025 Efterår");
        assert!(config.active);
        assert_eq!(config.channels.len(), 1);
    }

    #[test]
    fn test_parse_users() {
        let json = r#"[
            {"Name": "Test User", "DiscordId": "test-uuid-123"}
        ]"#;

        let users: Vec<SeasonUser> = serde_json::from_str(json).unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].name, "Test User");
        assert_eq!(users[0].id, "test-uuid-123");
    }
}
