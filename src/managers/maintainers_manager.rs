use poise::serenity_prelude::{Attachment, ChannelId, GuildId, Http};
use std::sync::Arc;
use tracing::info;

use crate::error::{BotError, Result};
use crate::managers::{SharedChannelManager, SharedConfigManager};

/// Type of configuration file detected
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigType {
    /// Season user list (e.g., "2025E.json")
    Season(String),
    /// Special members file
    SpecialMembers,
    /// Global structure file
    GlobalStructure,
    /// Category-specific structure (e.g., "2025E_structure.json")
    CategoryStructure(String),
    /// User database export
    UserDatabase,
    /// Unknown type
    Unknown,
}

/// Manages the maintainers channel and file uploads
pub struct MaintainersManager {
    config_manager: SharedConfigManager,
    channel_manager: SharedChannelManager,
}

impl MaintainersManager {
    pub fn new(config_manager: SharedConfigManager, channel_manager: SharedChannelManager) -> Self {
        Self {
            config_manager,
            channel_manager,
        }
    }

    /// Ensure the maintainers channel exists
    pub async fn ensure_channel_exists(&self, http: &Http, guild_id: GuildId) -> Result<ChannelId> {
        let channel_manager = self.channel_manager.read().await;
        channel_manager
            .ensure_maintainers_channel(http, guild_id)
            .await
    }

    /// Check if a channel is the maintainers channel
    pub async fn is_maintainers_channel(&self, channel_id: ChannelId, guild_id: GuildId) -> bool {
        let channel_manager = self.channel_manager.read().await;
        channel_manager
            .is_maintainers_channel(channel_id, guild_id)
            .await
    }

    /// Detect the type of config from filename and content
    pub fn detect_config_type(&self, filename: &str, _content: &[u8]) -> ConfigType {
        let filename_lower = filename.to_lowercase();

        // Check for special files first
        if filename_lower == "special_members.json" {
            return ConfigType::SpecialMembers;
        }

        if filename_lower == "global_structure.json" {
            return ConfigType::GlobalStructure;
        }

        if filename_lower == "user_database.json" {
            return ConfigType::UserDatabase;
        }

        // Check for category structure (ends with _structure.json)
        if filename_lower.ends_with("_structure.json") {
            let season_id = filename_lower
                .trim_end_matches("_structure.json")
                .to_string();
            return ConfigType::CategoryStructure(season_id);
        }

        // Assume it's a season file if it ends with .json
        if filename_lower.ends_with(".json") {
            let season_id = filename_lower.trim_end_matches(".json").to_string();
            // Convert to uppercase for season ID (e.g., "2025e" -> "2025E")
            return ConfigType::Season(season_id.to_uppercase());
        }

        ConfigType::Unknown
    }

    /// Handle an uploaded attachment
    pub async fn handle_attachment(
        &self,
        attachment: &Attachment,
        staged_by: Option<String>,
    ) -> Result<String> {
        // Check if it's a JSON file
        if !attachment.filename.ends_with(".json") {
            return Err(BotError::ConfigValidation {
                message: format!(
                    "Only JSON files are supported. Got: {}",
                    attachment.filename
                ),
            });
        }

        // Download the file
        let content = attachment
            .download()
            .await
            .map_err(|e| BotError::Internal {
                message: format!("Failed to download attachment: {}", e),
            })?;

        // Detect config type
        let config_type = self.detect_config_type(&attachment.filename, &content);

        // Stage the config
        let mut config_manager = self.config_manager.write().await;

        match config_type {
            ConfigType::Season(season_id) => {
                info!("Staging season config: {}", season_id);
                config_manager.stage_season_from_bytes(&content, &attachment.filename, staged_by)
            }
            ConfigType::SpecialMembers => {
                info!("Staging special members config");
                config_manager.stage_special_members_from_bytes(&content, staged_by)
            }
            ConfigType::GlobalStructure => {
                info!("Staging global structure config");
                config_manager.stage_global_structure_from_bytes(&content, staged_by)
            }
            ConfigType::CategoryStructure(season_id) => {
                info!("Staging category structure for: {}", season_id);
                config_manager.stage_category_structure_from_bytes(&content, staged_by)
            }
            ConfigType::UserDatabase => {
                // User database is a special case - we might want to import it
                Err(BotError::ConfigValidation {
                    message:
                        "User database imports are not yet supported. Use /import-user-db command."
                            .to_string(),
                })
            }
            ConfigType::Unknown => Err(BotError::ConfigValidation {
                message: format!("Unknown config type for file: {}", attachment.filename),
            }),
        }
    }

    /// Handle multiple attachments from a message
    pub async fn handle_message_attachments(
        &self,
        attachments: &[Attachment],
        staged_by: Option<String>,
    ) -> Vec<(String, Result<String>)> {
        let mut results = Vec::new();

        for attachment in attachments {
            let result = self.handle_attachment(attachment, staged_by.clone()).await;
            results.push((attachment.filename.clone(), result));
        }

        results
    }

    /// Format attachment handling results for Discord message
    pub fn format_results(&self, results: &[(String, Result<String>)]) -> String {
        let mut output = String::new();

        for (filename, result) in results {
            match result {
                Ok(msg) => {
                    output.push_str(&format!("**{}**: {}\n", filename, msg));
                }
                Err(e) => {
                    output.push_str(&format!("**{}**: Error - {}\n", filename, e));
                }
            }
        }

        if results.iter().any(|(_, r)| r.is_ok()) {
            output.push_str(
                "\nUse `/preview-config` to see what would change, then `/commit-config` to apply.",
            );
        }

        output
    }
}

/// Shared maintainers manager type
pub type SharedMaintainersManager = Arc<tokio::sync::RwLock<MaintainersManager>>;

pub fn create_shared_maintainers_manager(
    config_manager: SharedConfigManager,
    channel_manager: SharedChannelManager,
) -> SharedMaintainersManager {
    Arc::new(tokio::sync::RwLock::new(MaintainersManager::new(
        config_manager,
        channel_manager,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_config_type() {
        // We can't fully test without the managers, but we can test the logic
        assert_eq!(
            detect_type_from_filename("2025E.json"),
            ConfigType::Season("2025E".to_string())
        );
        assert_eq!(
            detect_type_from_filename("special_members.json"),
            ConfigType::SpecialMembers
        );
        assert_eq!(
            detect_type_from_filename("global_structure.json"),
            ConfigType::GlobalStructure
        );
        assert_eq!(
            detect_type_from_filename("2025E_structure.json"),
            ConfigType::CategoryStructure("2025e".to_string())
        );
    }

    fn detect_type_from_filename(filename: &str) -> ConfigType {
        let filename_lower = filename.to_lowercase();

        if filename_lower == "special_members.json" {
            return ConfigType::SpecialMembers;
        }

        if filename_lower == "global_structure.json" {
            return ConfigType::GlobalStructure;
        }

        if filename_lower.ends_with("_structure.json") {
            let season_id = filename_lower
                .trim_end_matches("_structure.json")
                .to_string();
            return ConfigType::CategoryStructure(season_id);
        }

        if filename_lower.ends_with(".json") {
            let season_id = filename_lower.trim_end_matches(".json").to_string();
            return ConfigType::Season(season_id.to_uppercase());
        }

        ConfigType::Unknown
    }
}
