use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{SeasonConfig, SpecialMembersConfig, GlobalStructureConfig, CategoryStructureConfig};

/// Staged configuration waiting to be committed
#[derive(Debug, Clone, Default)]
pub struct StagedConfig {
    /// Staged season configs (season_id -> config)
    pub seasons: HashMap<String, SeasonConfig>,

    /// Staged special members config
    pub special_members: Option<SpecialMembersConfig>,

    /// Staged global structure config
    pub global_structure: Option<GlobalStructureConfig>,

    /// Staged category structures (season_id -> config)
    pub category_structures: HashMap<String, CategoryStructureConfig>,

    /// When the config was staged
    pub staged_at: u64,

    /// Discord user ID who staged the config
    pub staged_by: Option<String>,
}

impl StagedConfig {
    pub fn new() -> Self {
        Self {
            staged_at: current_timestamp(),
            ..Default::default()
        }
    }

    /// Check if there's anything staged
    pub fn is_empty(&self) -> bool {
        self.seasons.is_empty()
            && self.special_members.is_none()
            && self.global_structure.is_none()
            && self.category_structures.is_empty()
    }

    /// Stage a season config
    pub fn stage_season(&mut self, config: SeasonConfig, staged_by: Option<String>) {
        self.seasons.insert(config.season_id.clone(), config);
        self.staged_at = current_timestamp();
        self.staged_by = staged_by;
    }

    /// Stage special members config
    pub fn stage_special_members(&mut self, config: SpecialMembersConfig, staged_by: Option<String>) {
        self.special_members = Some(config);
        self.staged_at = current_timestamp();
        self.staged_by = staged_by;
    }

    /// Stage global structure config
    pub fn stage_global_structure(&mut self, config: GlobalStructureConfig, staged_by: Option<String>) {
        self.global_structure = Some(config);
        self.staged_at = current_timestamp();
        self.staged_by = staged_by;
    }

    /// Stage a category structure config
    pub fn stage_category_structure(&mut self, config: CategoryStructureConfig, staged_by: Option<String>) {
        self.category_structures.insert(config.season_id.clone(), config);
        self.staged_at = current_timestamp();
        self.staged_by = staged_by;
    }

    /// Clear all staged configs
    pub fn clear(&mut self) {
        self.seasons.clear();
        self.special_members = None;
        self.global_structure = None;
        self.category_structures.clear();
        self.staged_at = 0;
        self.staged_by = None;
    }

    /// Get a summary of what's staged
    pub fn get_summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.seasons.is_empty() {
            let season_ids: Vec<_> = self.seasons.keys().collect();
            parts.push(format!("Seasons: {}", season_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }

        if self.special_members.is_some() {
            parts.push("Special Members".to_string());
        }

        if self.global_structure.is_some() {
            parts.push("Global Structure".to_string());
        }

        if !self.category_structures.is_empty() {
            let category_ids: Vec<_> = self.category_structures.keys().collect();
            parts.push(format!("Category Structures: {}", category_ids.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }

        if parts.is_empty() {
            "Nothing staged".to_string()
        } else {
            parts.join("\n")
        }
    }
}

/// Diff between current and staged configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigDiff {
    pub additions: Vec<ConfigChange>,
    pub modifications: Vec<ConfigChange>,
    pub deletions: Vec<ConfigChange>,
}

impl ConfigDiff {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.modifications.is_empty() && self.deletions.is_empty()
    }

    pub fn add_addition(&mut self, change: ConfigChange) {
        self.additions.push(change);
    }

    pub fn add_modification(&mut self, change: ConfigChange) {
        self.modifications.push(change);
    }

    pub fn add_deletion(&mut self, change: ConfigChange) {
        self.deletions.push(change);
    }

    /// Format the diff for display
    pub fn format_for_display(&self) -> String {
        let mut output = String::new();

        if !self.additions.is_empty() {
            output.push_str("**Additions:**\n");
            for change in &self.additions {
                output.push_str(&format!("+ {} ({}): {}\n", change.entity_type, change.entity_name, change.details));
            }
            output.push('\n');
        }

        if !self.modifications.is_empty() {
            output.push_str("**Modifications:**\n");
            for change in &self.modifications {
                output.push_str(&format!("~ {} ({}): {}\n", change.entity_type, change.entity_name, change.details));
            }
            output.push('\n');
        }

        if !self.deletions.is_empty() {
            output.push_str("**Deletions:**\n");
            for change in &self.deletions {
                output.push_str(&format!("- {} ({}): {}\n", change.entity_type, change.entity_name, change.details));
            }
        }

        if output.is_empty() {
            "No changes detected".to_string()
        } else {
            output
        }
    }
}

/// A single configuration change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    pub change_type: ConfigChangeType,
    pub entity_type: String,  // "season", "role", "channel", "user", etc.
    pub entity_name: String,
    pub details: String,
}

impl ConfigChange {
    pub fn new(change_type: ConfigChangeType, entity_type: &str, entity_name: &str, details: &str) -> Self {
        Self {
            change_type,
            entity_type: entity_type.to_string(),
            entity_name: entity_name.to_string(),
            details: details.to_string(),
        }
    }
}

/// Type of configuration change
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigChangeType {
    Add,
    Modify,
    Remove,
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
    fn test_staged_config_summary() {
        let mut staged = StagedConfig::new();
        assert_eq!(staged.get_summary(), "Nothing staged");

        staged.stage_season(
            SeasonConfig {
                season_id: "2025E".to_string(),
                name: "Test".to_string(),
                active: true,
                users: vec![],
            },
            None,
        );

        assert!(staged.get_summary().contains("2025E"));
    }

    #[test]
    fn test_config_diff_display() {
        let mut diff = ConfigDiff::new();
        diff.add_addition(ConfigChange::new(
            ConfigChangeType::Add,
            "season",
            "2025E",
            "New season with 50 users",
        ));

        let display = diff.format_for_display();
        assert!(display.contains("Additions"));
        assert!(display.contains("2025E"));
    }
}
