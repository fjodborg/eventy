use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// State tracking for Discord channels, categories, and roles
/// Used to minimize API calls by caching what's already been created
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ChannelState {
    /// Schema version
    pub version: u32,

    /// Last sync timestamp
    pub last_synced: u64,

    /// Per-guild state (guild ID -> state)
    pub guilds: HashMap<String, GuildChannelState>,
}

impl ChannelState {
    pub fn new() -> Self {
        Self {
            version: 1,
            last_synced: current_timestamp(),
            guilds: HashMap::new(),
        }
    }

    /// Load from file or create new
    pub async fn load(path: &str) -> crate::error::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                serde_json::from_str(&content).map_err(|e| crate::error::BotError::ConfigParse {
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

    /// Save to file atomically
    pub async fn save(&self, path: &str) -> crate::error::Result<()> {
        let content = serde_json::to_string_pretty(self)?;

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

    /// Get or create guild state
    pub fn get_guild_mut(&mut self, guild_id: &str, guild_name: &str) -> &mut GuildChannelState {
        self.guilds
            .entry(guild_id.to_string())
            .or_insert_with(|| GuildChannelState::new(guild_id, guild_name))
    }

    /// Get guild state (read-only)
    pub fn get_guild(&self, guild_id: &str) -> Option<&GuildChannelState> {
        self.guilds.get(guild_id)
    }

    /// Check if an entity needs sync (doesn't exist or is outdated)
    pub fn needs_sync(&self, guild_id: &str, entity_type: EntityType, name: &str) -> bool {
        match self.guilds.get(guild_id) {
            None => true,
            Some(guild) => match entity_type {
                EntityType::Category => !guild.categories.contains_key(name),
                EntityType::Channel => !guild.channels.contains_key(name),
                EntityType::Role => !guild.roles.contains_key(name),
            },
        }
    }

    /// Update last synced timestamp
    pub fn mark_synced(&mut self) {
        self.last_synced = current_timestamp();
    }
}

/// Entity type for sync checking
#[derive(Debug, Clone, Copy)]
pub enum EntityType {
    Category,
    Channel,
    Role,
}

/// State for a single guild
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildChannelState {
    pub guild_id: String,
    pub guild_name: String,

    /// Categories (name -> state)
    pub categories: HashMap<String, CategoryState>,

    /// Channels (name -> state)
    pub channels: HashMap<String, ChannelStateEntry>,

    /// Roles (name -> state)
    pub roles: HashMap<String, RoleState>,

    /// ID of the maintainers channel if it exists
    pub maintainers_channel_id: Option<String>,
}

impl GuildChannelState {
    pub fn new(guild_id: &str, guild_name: &str) -> Self {
        Self {
            guild_id: guild_id.to_string(),
            guild_name: guild_name.to_string(),
            categories: HashMap::new(),
            channels: HashMap::new(),
            roles: HashMap::new(),
            maintainers_channel_id: None,
        }
    }

    /// Record a created category
    pub fn add_category(&mut self, name: &str, discord_id: &str, position: u16) {
        self.categories.insert(
            name.to_string(),
            CategoryState {
                discord_id: discord_id.to_string(),
                name: name.to_string(),
                position,
                created_by_bot: true,
                last_verified: current_timestamp(),
            },
        );
    }

    /// Record a created channel
    pub fn add_channel(
        &mut self,
        name: &str,
        discord_id: &str,
        parent: Option<&str>,
        channel_type: &str,
    ) {
        self.channels.insert(
            name.to_string(),
            ChannelStateEntry {
                discord_id: discord_id.to_string(),
                name: name.to_string(),
                parent_category: parent.map(String::from),
                channel_type: channel_type.to_string(),
                created_by_bot: true,
                permissions_applied: true,
                last_verified: current_timestamp(),
            },
        );
    }

    /// Record a created role
    pub fn add_role(&mut self, name: &str, discord_id: &str, color: Option<&str>) {
        self.roles.insert(
            name.to_string(),
            RoleState {
                discord_id: discord_id.to_string(),
                name: name.to_string(),
                color: color.map(String::from),
                created_by_bot: true,
                permissions_applied: true,
                last_verified: current_timestamp(),
            },
        );
    }

    /// Get category ID by name
    pub fn get_category_id(&self, name: &str) -> Option<&str> {
        self.categories.get(name).map(|c| c.discord_id.as_str())
    }

    /// Get channel ID by name
    pub fn get_channel_id(&self, name: &str) -> Option<&str> {
        self.channels.get(name).map(|c| c.discord_id.as_str())
    }

    /// Get role ID by name
    pub fn get_role_id(&self, name: &str) -> Option<&str> {
        self.roles.get(name).map(|r| r.discord_id.as_str())
    }

    /// Set maintainers channel ID
    pub fn set_maintainers_channel(&mut self, channel_id: &str) {
        self.maintainers_channel_id = Some(channel_id.to_string());
    }
}

/// State for a category
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryState {
    pub discord_id: String,
    pub name: String,
    pub position: u16,
    pub created_by_bot: bool,
    pub last_verified: u64,
}

/// State for a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelStateEntry {
    pub discord_id: String,
    pub name: String,
    pub parent_category: Option<String>,
    pub channel_type: String,
    pub created_by_bot: bool,
    pub permissions_applied: bool,
    pub last_verified: u64,
}

/// State for a role
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleState {
    pub discord_id: String,
    pub name: String,
    pub color: Option<String>,
    pub created_by_bot: bool,
    pub permissions_applied: bool,
    pub last_verified: u64,
}

/// Shared channel state type
pub type SharedChannelState = Arc<tokio::sync::RwLock<ChannelState>>;

pub fn create_shared_channel_state(state: ChannelState) -> SharedChannelState {
    Arc::new(tokio::sync::RwLock::new(state))
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
    fn test_guild_state_operations() {
        let mut state = ChannelState::new();

        let guild = state.get_guild_mut("123", "Test Guild");
        guild.add_role("Medlem", "456", Some("#00ff00"));
        guild.add_category("2025E", "789", 0);
        guild.add_channel("general", "101", Some("2025E"), "text");

        assert!(state.get_guild("123").is_some());
        assert_eq!(
            state.get_guild("123").unwrap().get_role_id("Medlem"),
            Some("456")
        );
    }

    #[test]
    fn test_needs_sync() {
        let mut state = ChannelState::new();
        let guild = state.get_guild_mut("123", "Test");
        guild.add_role("Medlem", "456", None);

        assert!(!state.needs_sync("123", EntityType::Role, "Medlem"));
        assert!(state.needs_sync("123", EntityType::Role, "Admin"));
        assert!(state.needs_sync("999", EntityType::Role, "Medlem"));
    }
}
