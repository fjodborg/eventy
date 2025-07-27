// src/channel_manager.rs
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serenity::{
    all::{CreateChannel, PermissionOverwrite, PermissionOverwriteType}, http::Http, model::{
        channel::{ChannelType, GuildChannel},
        guild::Guild,
        id::{ChannelId, GuildId, RoleId},
        permissions::Permissions,
    }
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

pub type SharedChannelManager = Arc<RwLock<ChannelManager>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ConfigChannelType {
    Text,
    Voice,
    Category,
    News,
    Stage,
    Forum,
}

impl From<ConfigChannelType> for ChannelType {
    fn from(config_type: ConfigChannelType) -> Self {
        match config_type {
            ConfigChannelType::Text => ChannelType::Text,
            ConfigChannelType::Voice => ChannelType::Voice,
            ConfigChannelType::Category => ChannelType::Category,
            ConfigChannelType::News => ChannelType::News,
            ConfigChannelType::Stage => ChannelType::Stage,
            ConfigChannelType::Forum => ChannelType::Forum,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub channel_type: ConfigChannelType,
    pub permissions: HashMap<String, bool>,
    pub position: Option<u16>,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>, // Sub-channels for categories
}

#[derive(Debug)]
pub struct ChannelManager {
    configs: Vec<ChannelConfig>,
    // Cache: guild_id -> (channel_name -> channel_id)
    channel_cache: DashMap<GuildId, HashMap<String, ChannelId>>,
    // Cache: guild_id -> (category_name -> category_id)
    category_cache: DashMap<GuildId, HashMap<String, ChannelId>>,
}

impl ChannelManager {
    pub fn new(configs: Vec<ChannelConfig>) -> Self {
        Self {
            configs,
            channel_cache: DashMap::new(),
            category_cache: DashMap::new(),
        }
    }

    pub async fn load_from_file(file_path: &str) -> Result<Self> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let configs: Vec<ChannelConfig> = serde_json::from_str(&content)?;
        Ok(Self::new(configs))
    }

    pub async fn reload_config(&mut self, file_path: &str) -> Result<()> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let configs: Vec<ChannelConfig> = serde_json::from_str(&content)?;
        self.configs = configs;
        // Clear caches to force refresh
        self.channel_cache.clear();
        self.category_cache.clear();
        info!("Channel configuration reloaded from {}", file_path);
        Ok(())
    }

    pub fn get_configs(&self) -> &Vec<ChannelConfig> {
        &self.configs
    }

    /// Updates the internal cache with current guild channels
    pub async fn update_cache(&self, http: &Http, guild_id: GuildId) -> Result<()> {
        let channels = guild_id.channels(http).await?;
        
        let mut channel_map = HashMap::new();
        let mut category_map = HashMap::new();

        for (channel_id, channel) in channels {
            match channel.kind {
                ChannelType::Category => {
                    category_map.insert(channel.name.clone(), channel_id);
                }
                _ => {
                    channel_map.insert(channel.name.clone(), channel_id);
                }
            }
        }

        self.channel_cache.insert(guild_id, channel_map);
        self.category_cache.insert(guild_id, category_map);
        
        debug!("Updated channel cache for guild {}", guild_id);
        Ok(())
    }

    /// Ensures all configured channels and categories exist in the guild
    pub async fn ensure_channels_exist(&self, http: &Http, guild_id: GuildId) -> Result<()> {
        // Update cache first
        self.update_cache(http, guild_id).await?;

        // Get guild roles for permission setup
        let guild_roles = guild_id.roles(http).await?;

        for config in &self.configs {
            match config.channel_type {
                ConfigChannelType::Category => {
                    // Handle category creation
                    let category_id = self.ensure_category_exists(http, guild_id, config, &guild_roles).await?;
                    
                    // Handle sub-channels within the category
                    for sub_channel_config in &config.channels {
                        self.ensure_sub_channel_exists(
                            http, 
                            guild_id, 
                            sub_channel_config, 
                            Some(category_id),
                            config, // Parent category config for inherited permissions
                            &guild_roles
                        ).await?;
                    }
                }
                _ => {
                    // Handle standalone channels
                    self.ensure_channel_exists(http, guild_id, config, None, &guild_roles).await?;
                }
            }
        }

        Ok(())
    }

    async fn ensure_category_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        config: &ChannelConfig,
        guild_roles: &HashMap<RoleId, serenity::model::guild::Role>,
    ) -> Result<ChannelId> {
        // Check if category exists in cache
        if let Some(category_cache) = self.category_cache.get(&guild_id) {
            if let Some(&category_id) = category_cache.get(&config.name) {
                debug!("Category '{}' already exists with ID: {}", config.name, category_id);
                return Ok(category_id);
            }
        }

        info!("Creating category: {}", config.name);

        let mut create_channel = CreateChannel::new(&config.name)
            .kind(ChannelType::Category);

        if let Some(position) = config.position {
            create_channel = create_channel.position(position);
        }

        // Set permissions
        let permission_overwrites = self.build_permission_overwrites(&config.permissions, guild_roles)?;
        create_channel = create_channel.permissions(permission_overwrites.into_iter());

        let category = guild_id.create_channel(http, create_channel).await?;
        
        // Update cache
        if let Some(mut category_cache) = self.category_cache.get_mut(&guild_id) {
            category_cache.insert(config.name.clone(), category.id);
        }

        info!("Successfully created category: {} ({})", config.name, category.id);
        Ok(category.id)
    }

    async fn ensure_channel_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        config: &ChannelConfig,
        parent_id: Option<ChannelId>,
        guild_roles: &HashMap<RoleId, serenity::model::guild::Role>,
    ) -> Result<ChannelId> {
        // Check if channel exists in cache
        if let Some(channel_cache) = self.channel_cache.get(&guild_id) {
            if let Some(&channel_id) = channel_cache.get(&config.name) {
                debug!("Channel '{}' already exists with ID: {}", config.name, channel_id);
                return Ok(channel_id);
            }
        }

        info!("Creating channel: {}", config.name);

        let mut create_channel = CreateChannel::new(&config.name)
            .kind(config.channel_type.clone().into());

        if let Some(parent_id) = parent_id {
            create_channel = create_channel.category(parent_id);
        }

        if let Some(position) = config.position {
            create_channel = create_channel.position(position);
        }

        // Set permissions
        let permission_overwrites = self.build_permission_overwrites(&config.permissions, guild_roles)?;
        create_channel = create_channel.permissions(permission_overwrites.into_iter());

        let channel = guild_id.create_channel(http, create_channel).await?;
        
        // Update cache
        if let Some(mut channel_cache) = self.channel_cache.get_mut(&guild_id) {
            channel_cache.insert(config.name.clone(), channel.id);
        }

        info!("Successfully created channel: {} ({})", config.name, channel.id);
        Ok(channel.id)
    }

    async fn ensure_sub_channel_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        sub_config: &ChannelConfig,
        parent_id: Option<ChannelId>,
        parent_config: &ChannelConfig,
        guild_roles: &HashMap<RoleId, serenity::model::guild::Role>,
    ) -> Result<ChannelId> {
        // Merge permissions: parent permissions + sub-channel specific permissions
        let mut merged_permissions = parent_config.permissions.clone();
        for (perm, value) in &sub_config.permissions {
            merged_permissions.insert(perm.clone(), *value);
        }

        let merged_config = ChannelConfig {
            name: sub_config.name.clone(),
            channel_type: sub_config.channel_type.clone(),
            permissions: merged_permissions,
            position: sub_config.position,
            channels: vec![], // Sub-channels don't have their own sub-channels
        };

        self.ensure_channel_exists(http, guild_id, &merged_config, parent_id, guild_roles).await
    }

    fn build_permission_overwrites(
        &self,
        permissions: &HashMap<String, bool>,
        guild_roles: &HashMap<RoleId, serenity::model::guild::Role>,
    ) -> Result<Vec<PermissionOverwrite>> {
        let mut overwrites = Vec::new();
        // TODO: clean up the channel code.
        let msg= "TODO:Channel permission overwrite is disabled. Use roles for now until code is cleanup";
        warn!(msg);
        anyhow::bail!(msg);
        // Find @everyone role
        let everyone_role = guild_roles
            .values()
            .find(|role| role.name == "@everyone")
            .ok_or_else(|| anyhow!("Could not find @everyone role"))?;

        let mut allow = Permissions::empty();
        let mut deny = Permissions::empty();

        for (perm_name, enabled) in permissions {
            let permission = match perm_name.as_str() {
                "view_channel" => Permissions::VIEW_CHANNEL,
                "send_messages" => Permissions::SEND_MESSAGES,
                "read_message_history" => Permissions::READ_MESSAGE_HISTORY,
                "add_reactions" => Permissions::ADD_REACTIONS,
                "attach_files" => Permissions::ATTACH_FILES,
                "embed_links" => Permissions::EMBED_LINKS,
                "use_external_emojis" => Permissions::USE_EXTERNAL_EMOJIS,
                "connect" => Permissions::CONNECT,
                "speak" => Permissions::SPEAK,
                "use_voice_activation" => Permissions::USE_VAD,
                "manage_messages" => Permissions::MANAGE_MESSAGES,
                "manage_channels" => Permissions::MANAGE_CHANNELS,
                _ => {
                    warn!("Unknown permission: {}", perm_name);
                    continue;
                }
            };

            if *enabled {
                allow |= permission;
            } else {
                deny |= permission;
            }
        }

        if !allow.is_empty() || !deny.is_empty() {
            overwrites.push(
                PermissionOverwrite{
                    allow,
                    deny,
                    kind: PermissionOverwriteType::Role(everyone_role.id)
                }
            );
        }

        Ok(overwrites)
    }

    pub fn find_channel_id(&self, guild_id: GuildId, channel_name: &str) -> Option<ChannelId> {
        self.channel_cache
            .get(&guild_id)?
            .get(channel_name)
            .copied()
    }

    pub fn find_category_id(&self, guild_id: GuildId, category_name: &str) -> Option<ChannelId> {
        self.category_cache
            .get(&guild_id)?
            .get(category_name)
            .copied()
    }
}

pub async fn create_shared_channel_manager(config_file: &str) -> Result<SharedChannelManager> {
    let manager = ChannelManager::load_from_file(config_file).await?;
    Ok(Arc::new(RwLock::new(manager)))
}