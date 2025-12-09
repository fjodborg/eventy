use super::global_structure::{ChannelDefinition, RoleDefinition};
use serde::{Deserialize, Serialize};

/// Per-season category structure configuration
/// Overrides or extends the global structure for a specific season
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryStructureConfig {
    /// Season ID this structure applies to (e.g., "2025E")
    /// Optional because it might be inferred from the directory
    #[serde(default)]
    pub season_id: String,

    /// Name for the Discord category (defaults to season_id if not specified)
    #[serde(default)]
    pub category_name: Option<String>,

    /// Full list of channels for this category (if provided, replaces global defaults)
    #[serde(default)]
    pub channels: Vec<ChannelDefinition>,

    /// Full list of roles for this category (if provided, replaces global defaults)
    #[serde(default)]
    pub roles: Vec<RoleDefinition>,

    /// Role overrides for this season (replaces roles with same name from global)
    #[serde(default)]
    pub role_overrides: Vec<RoleDefinition>,

    /// Channel overrides for this season (replaces channels with same name from global)
    #[serde(default)]
    pub channel_overrides: Vec<ChannelDefinition>,

    /// Additional channels specific to this season (added to global channels)
    #[serde(default)]
    pub additional_channels: Vec<ChannelDefinition>,

    /// Additional roles specific to this season
    #[serde(default)]
    pub additional_roles: Vec<RoleDefinition>,
}

impl CategoryStructureConfig {
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

    /// Get the category name (falls back to season_id)
    pub fn get_category_name(&self) -> &str {
        self.category_name.as_deref().unwrap_or(&self.season_id)
    }

    /// Merge with global structure to get final configuration
    pub fn merge_with_global(
        &self,
        global: &super::global_structure::GlobalStructureConfig,
    ) -> MergedStructure {
        // If explicit roles are provided, use them. Otherwise merge overrides/additional with global.
        let roles = if !self.roles.is_empty() {
            self.roles.clone()
        } else {
            let mut roles = Vec::new();
            // Start with global roles
            for global_role in &global.default_roles {
                // Check if there's an override
                if let Some(override_role) = self
                    .role_overrides
                    .iter()
                    .find(|r| r.name == global_role.name)
                {
                    roles.push(override_role.clone());
                } else {
                    roles.push(global_role.clone());
                }
            }
            // Add additional roles
            for role in &self.additional_roles {
                if !roles.iter().any(|r| r.name == role.name) {
                    roles.push(role.clone());
                }
            }
            roles
        };

        // If explicit channels are provided, use them. Otherwise merge overrides/additional with global.
        let channels = if !self.channels.is_empty() {
            self.channels.clone()
        } else {
            let mut channels = Vec::new();
            // Start with global channels
            for global_channel in &global.default_channels {
                // Check if there's an override
                if let Some(override_channel) = self
                    .channel_overrides
                    .iter()
                    .find(|c| c.name == global_channel.name)
                {
                    channels.push(override_channel.clone());
                } else {
                    channels.push(global_channel.clone());
                }
            }
            // Add additional channels
            for channel in &self.additional_channels {
                if !channels.iter().any(|c| c.name == channel.name) {
                    channels.push(channel.clone());
                }
            }
            channels
        };

        MergedStructure {
            category_name: self.get_category_name().to_string(),
            roles,
            channels,
        }
    }
}

/// Result of merging category structure with global structure
#[derive(Debug, Clone)]
pub struct MergedStructure {
    pub category_name: String,
    pub roles: Vec<RoleDefinition>,
    pub channels: Vec<ChannelDefinition>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::global_structure::GlobalStructureConfig;

    #[test]
    fn test_merge_with_global() {
        let global = GlobalStructureConfig::default();

        let category_json = r#"{
            "season_id": "2025E",
            "category_name": "2025 Efterår",
            "additional_channels": [
                {
                    "name": "announcements",
                    "type": "text",
                    "role_permissions": {},
                    "children": []
                }
            ]
        }"#;

        let category: CategoryStructureConfig = serde_json::from_str(category_json).unwrap();
        let merged = category.merge_with_global(&global);

        assert_eq!(merged.category_name, "2025 Efterår");
        // Should have global channel + additional channel
        assert!(merged.channels.len() >= 2);
    }
}
